//! File use cases: uploads, content, renames, moves, deletes, and the capacity bar.

use std::sync::Arc;

use bytes::Bytes;
use futures_util::future::try_join;
use sha2::{Digest, Sha256};

use crate::domains::chat::ChatRepository;
use crate::domains::file::audio::duration_ms;
use crate::domains::file::capacity::Capacity;
use crate::domains::file::entities::{FileOwner, Measured, StoredFile};
use crate::domains::file::media::{FileKind, MediaType, estimate, pdf_pages};
use crate::domains::file::ports::FileRepository;
use crate::domains::project::ProjectRepository;
use crate::domains::provider::ModelRepository;
use crate::ports::clock::Clock;
use crate::ports::worker::{InlineWorker, Worker, offload};
use crate::shared::actor::Actor;
use crate::shared::constants::{
    AUDIO_CLIP_MAX_SECS, DEFAULT_MAX_STORAGE_MB_PER_USER, DEFAULT_MAX_UPLOAD_MB,
};
use crate::shared::errors::{DomainError, DomainResult};
use crate::shared::ids::{FileId, ProjectId};

const MEBIBYTE: u64 = 1024 * 1024;

/// Upload limits (`MAX_UPLOAD_MB` and `MAX_STORAGE_MB_PER_USER`).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FileLimits {
    pub max_upload_mb: u64,
    pub max_storage_mb: u64,
}

impl Default for FileLimits {
    fn default() -> Self {
        Self {
            max_upload_mb: DEFAULT_MAX_UPLOAD_MB,
            max_storage_mb: DEFAULT_MAX_STORAGE_MB_PER_USER,
        }
    }
}

/// Input of [`FileService::update`]; `None` keeps a field.
#[derive(Clone, Debug, Default)]
pub struct FileUpdate {
    pub name: Option<String>,
    pub included: Option<bool>,
}

#[derive(Clone)]
pub struct FileService {
    files: Arc<dyn FileRepository>,
    projects: Arc<dyn ProjectRepository>,
    chats: Arc<dyn ChatRepository>,
    models: Arc<dyn ModelRepository>,
    clock: Arc<dyn Clock>,
    limits: FileLimits,
    worker: Arc<dyn Worker>,
}

impl FileService {
    pub fn new(
        files: Arc<dyn FileRepository>,
        projects: Arc<dyn ProjectRepository>,
        chats: Arc<dyn ChatRepository>,
        models: Arc<dyn ModelRepository>,
        clock: Arc<dyn Clock>,
        limits: FileLimits,
    ) -> Self {
        Self {
            files,
            projects,
            chats,
            models,
            clock,
            limits,
            worker: Arc::new(InlineWorker),
        }
    }

    /// Measures uploads (audio length, PDF pages, SHA-256) on `worker` instead of the caller's
    /// task. The API passes Tokio's blocking pool; without it the work runs in place.
    pub fn with_worker(mut self, worker: Arc<dyn Worker>) -> Self {
        self.worker = worker;
        self
    }

    /// Stores an upload after checking, in order: the owner belongs to the caller, the size
    /// limit, the type (extension plus magic bytes), the length of audio for a chat (at most
    /// `AUDIO_CLIP_MAX_SECS`, and measurable), and the caller's storage quota. Records the SHA-256, the token estimate, and the page count of PDFs.
    pub async fn upload(
        &self,
        actor: &Actor,
        owner: FileOwner,
        name: &str,
        bytes: Bytes,
    ) -> DomainResult<StoredFile> {
        self.check_owner(actor, owner).await?;
        let size_bytes = bytes.len() as u64;
        if size_bytes == 0 {
            return Err(DomainError::validation("The file is empty.").with_field("file"));
        }
        if size_bytes > self.limits.max_upload_mb * MEBIBYTE {
            return Err(DomainError::validation(format!(
                "Files must be at most {} MB.",
                self.limits.max_upload_mb
            ))
            .with_field("file")
            .with_context("maxUploadMb", self.limits.max_upload_mb));
        }
        let media_type = MediaType::detect(name, &bytes)?;
        let clip = matches!(owner, FileOwner::Chat(_)) && media_type.kind() == FileKind::Audio;
        // Linear in the upload's size, which may be up to 1,000 MB: off the request's task.
        let content = bytes.clone();
        let (checked, page_count, sha256) = offload(&*self.worker, move || {
            let checked = if clip {
                check_clip(media_type, &content)
            } else {
                Ok(())
            };
            let pages = (media_type.kind() == FileKind::Pdf).then(|| pdf_pages(&content));
            let sha256: [u8; 32] = Sha256::digest(&content).into();
            (checked, pages, sha256)
        })
        .await;
        checked?;
        let quota = self.limits.max_storage_mb * MEBIBYTE;
        // Fails fast before storing; `create` checks again under the user's lock.
        let used = self.files.usage(actor.user_id).await?;
        if used.saturating_add(size_bytes) > quota {
            return Err(self.storage_full());
        }
        let measured = Measured {
            media_type,
            size_bytes,
            sha256,
            token_estimate: estimate(media_type, size_bytes, page_count),
            page_count,
        };
        let file = StoredFile::create(actor.user_id, owner, name, measured, self.clock.now())?;
        if !self.files.create(&file, bytes, quota).await? {
            // Uploads at once filled the room since `usage`.
            return Err(self.storage_full());
        }
        Ok(file)
    }

    fn storage_full(&self) -> DomainError {
        DomainError::business_rule(
            format!(
                "Your file storage is full ({} MB). Delete files to make room.",
                self.limits.max_storage_mb
            ),
            "STORAGE_FULL",
        )
        .with_context("maxStorageMb", self.limits.max_storage_mb)
    }

    /// A project's files, oldest first.
    pub async fn list(&self, actor: &Actor, project: ProjectId) -> DomainResult<Vec<StoredFile>> {
        self.check_owner(actor, FileOwner::Project(project)).await?;
        self.files
            .list(actor.user_id, FileOwner::Project(project))
            .await
    }

    pub async fn get(&self, actor: &Actor, id: FileId) -> DomainResult<StoredFile> {
        self.files
            .get(actor.user_id, id)
            .await?
            .ok_or_else(|| not_found(id))
    }

    /// The file and its bytes (two reads at once). Serve them with
    /// `served_type(file.media_type())`.
    pub async fn content(&self, actor: &Actor, id: FileId) -> DomainResult<(StoredFile, Bytes)> {
        let (file, bytes) = try_join(
            self.files.get(actor.user_id, id),
            self.files.content(actor.user_id, id),
        )
        .await?;
        match (file, bytes) {
            (Some(file), Some(bytes)) => Ok((file, bytes)),
            _ => Err(not_found(id)),
        }
    }

    /// The file's bytes alone, in one read, for a caller that already has its metadata.
    pub async fn bytes(&self, actor: &Actor, id: FileId) -> DomainResult<Bytes> {
        self.files
            .content(actor.user_id, id)
            .await?
            .ok_or_else(|| not_found(id))
    }

    /// At most `len` bytes of the file from byte `offset`, for a download that sends the file in
    /// pieces; empty past its end.
    pub async fn piece(
        &self,
        actor: &Actor,
        id: FileId,
        offset: u64,
        len: u64,
    ) -> DomainResult<Bytes> {
        self.files
            .piece(actor.user_id, id, offset, len)
            .await?
            .ok_or_else(|| not_found(id))
    }

    /// Renames the file or includes it in (or leaves it out of) the project context.
    pub async fn update(
        &self,
        actor: &Actor,
        id: FileId,
        changes: FileUpdate,
    ) -> DomainResult<StoredFile> {
        let mut file = self.get(actor, id).await?;
        if let Some(name) = &changes.name {
            file = file.with_name(name)?;
        }
        if let Some(included) = changes.included {
            file = file.with_included(included)?;
        }
        self.files.update(&file).await?;
        Ok(file)
    }

    /// Moves a file (a chat attachment's "Add to project", or a project file) into `project`.
    /// No copy and no re-upload.
    pub async fn move_to_project(
        &self,
        actor: &Actor,
        id: FileId,
        project: ProjectId,
    ) -> DomainResult<StoredFile> {
        let file = self.get(actor, id).await?;
        self.check_owner(actor, FileOwner::Project(project)).await?;
        if file.owner() == FileOwner::Project(project) {
            return Ok(file);
        }
        self.files.move_to(actor.user_id, &[id], project).await?;
        Ok(file.moved(project))
    }

    pub async fn delete(&self, actor: &Actor, id: FileId) -> DomainResult<()> {
        if self.files.delete(actor.user_id, id).await? {
            Ok(())
        } else {
            Err(not_found(id))
        }
    }

    /// The project's included files against its default model's context window.
    pub async fn capacity(&self, actor: &Actor, project: ProjectId) -> DomainResult<Capacity> {
        let (stored, files) = try_join(
            self.projects.get(actor.user_id, project),
            self.files.list(actor.user_id, FileOwner::Project(project)),
        )
        .await?;
        let stored = stored.ok_or_else(|| project_not_found(project))?;
        let window = match stored.default_model_id() {
            Some(model) => self
                .models
                .get(actor.user_id, model)
                .await?
                .and_then(|model| model.context_window()),
            None => None,
        };
        Ok(Capacity::measure(&files, window))
    }

    /// The owner must be one of the caller's projects or chats (404 otherwise).
    async fn check_owner(&self, actor: &Actor, owner: FileOwner) -> DomainResult<()> {
        match owner {
            FileOwner::Project(project) => self
                .projects
                .get(actor.user_id, project)
                .await?
                .map(|_| ())
                .ok_or_else(|| project_not_found(project)),
            FileOwner::Chat(chat) => self
                .chats
                .get(actor.user_id, chat)
                .await?
                .map(|_| ())
                .ok_or_else(|| DomainError::not_found("Chat", chat.to_string())),
        }
    }
}

/// Audio a chat sends to a model plays for at most `AUDIO_CLIP_MAX_SECS`, and a file whose
/// length cannot be measured is refused rather than trusted. Project audio never reaches a
/// model, so only the upload size limits it.
fn check_clip(media: MediaType, bytes: &[u8]) -> DomainResult<()> {
    let limit = u64::from(AUDIO_CLIP_MAX_SECS) * 1_000;
    match duration_ms(media, bytes) {
        Some(played) if played <= limit => Ok(()),
        Some(_) => Err(DomainError::validation(format!(
            "Audio in a chat can be at most {} minutes long.",
            AUDIO_CLIP_MAX_SECS / 60
        ))
        .with_field("file")
        .with_context("maxSeconds", AUDIO_CLIP_MAX_SECS)),
        None => Err(DomainError::validation(
            "We could not tell how long this audio plays. Send a PCM WAV or an MP3 file.",
        )
        .with_field("file")
        .with_context("maxSeconds", AUDIO_CLIP_MAX_SECS)),
    }
}

fn not_found(id: FileId) -> DomainError {
    DomainError::not_found("File", id.to_string())
}

fn project_not_found(id: ProjectId) -> DomainError {
    DomainError::not_found("Project", id.to_string())
}

#[cfg(test)]
mod tests {
    use chrono::Utc;

    use super::*;
    use crate::domains::chat::Chat;
    use crate::domains::file::capacity::CapacityLevel;
    use crate::domains::project::{NewProject, ProjectService, ProjectUpdate};
    use crate::domains::provider::{
        ModelQuery, ModelService, ModelUpdate, NewProvider, Preset, ProviderKind, ProviderService,
    };
    use crate::shared::ids::{ChatId, UserId};
    use crate::shared::net::PrivateNetwork;
    use crate::testing::{
        FakeCipher, FixedClock, MemoryStore, ScriptedGateway, actor, remote_model,
    };

    const PNG: &[u8] = b"\x89PNG\r\n\x1a\n....";

    struct Fixture {
        files: FileService,
        projects: ProjectService,
        providers: ProviderService,
        models: ModelService,
        gateway: Arc<ScriptedGateway>,
        store: Arc<MemoryStore>,
    }

    fn fixture(limits: FileLimits) -> Fixture {
        let clock = Arc::new(FixedClock::default());
        let store = Arc::new(MemoryStore::new(clock.clone()));
        let gateway = Arc::new(ScriptedGateway::new());
        Fixture {
            files: FileService::new(
                store.clone(),
                store.clone(),
                store.clone(),
                store.clone(),
                clock.clone(),
                limits,
            ),
            projects: ProjectService::new(
                store.clone(),
                store.clone(),
                store.clone(),
                clock.clone(),
            ),
            providers: ProviderService::new(
                store.clone(),
                store.clone(),
                store.clone(),
                gateway.clone(),
                Arc::new(FakeCipher::new()),
                clock.clone(),
                PrivateNetwork::Admins,
            ),
            models: ModelService::new(store.clone(), store.clone(), store.clone(), clock),
            gateway,
            store,
        }
    }

    async fn new_project(f: &Fixture, caller: &Actor) -> ProjectId {
        let input = NewProject {
            name: "Work".to_owned(),
            ..NewProject::default()
        };
        f.projects.create(caller, input).await.unwrap().id()
    }

    fn text(content: &str) -> Bytes {
        Bytes::copy_from_slice(content.as_bytes())
    }

    fn is_not_found(error: &DomainError, entity: &str) -> bool {
        matches!(error, DomainError::EntityNotFound { entity_name, .. } if entity_name == entity)
    }

    #[tokio::test]
    async fn uploads_a_text_file_with_its_measurements() {
        let f = fixture(FileLimits::default());
        let caller = actor();
        let project = new_project(&f, &caller).await;
        let content = "fn main() {}\n";
        let file = f
            .files
            .upload(
                &caller,
                FileOwner::Project(project),
                "src/main.rs",
                text(content),
            )
            .await
            .unwrap();
        assert_eq!(file.name(), "main.rs");
        assert_eq!(file.media_type(), "text/plain");
        assert_eq!(file.size_bytes(), 13);
        assert_eq!(file.token_estimate(), 4);
        assert_eq!(file.page_count(), None);
        assert!(file.included());
        let expected: [u8; 32] = Sha256::digest(content.as_bytes()).into();
        assert_eq!(file.sha256(), &expected);

        let (stored, bytes) = f.files.content(&caller, file.id()).await.unwrap();
        assert_eq!(stored, file);
        assert_eq!(bytes, text(content));
        assert_eq!(f.files.list(&caller, project).await.unwrap(), [file]);
    }

    #[tokio::test]
    async fn counts_pdf_pages_and_estimates_them() {
        let f = fixture(FileLimits::default());
        let caller = actor();
        let project = new_project(&f, &caller).await;
        let pdf =
            Bytes::from_static(b"%PDF-1.7\n<< /Type /Page >> << /Type /Page >> << /Type /Pages >>");
        let file = f
            .files
            .upload(&caller, FileOwner::Project(project), "paper.pdf", pdf)
            .await
            .unwrap();
        assert_eq!(file.media_type(), "application/pdf");
        assert_eq!(file.page_count(), Some(2));
        assert_eq!(file.token_estimate(), 6_000);
    }

    #[tokio::test]
    async fn rejects_unsupported_and_mismatched_types() {
        let f = fixture(FileLimits::default());
        let caller = actor();
        let project = new_project(&f, &caller).await;
        for (name, bytes) in [
            ("logo.svg", text("<svg/>")),
            ("photo.png", text("not a png")),
        ] {
            let error = f
                .files
                .upload(&caller, FileOwner::Project(project), name, bytes)
                .await
                .unwrap_err();
            assert_eq!(
                error.to_string(),
                "This file type is not supported.",
                "{name}"
            );
        }
    }

    #[tokio::test]
    async fn rejects_empty_and_oversized_files() {
        let limits = FileLimits {
            max_upload_mb: 1,
            max_storage_mb: 10,
        };
        let f = fixture(limits);
        let caller = actor();
        let owner = FileOwner::Project(new_project(&f, &caller).await);
        let error = f
            .files
            .upload(&caller, owner, "a.txt", Bytes::new())
            .await
            .unwrap_err();
        assert_eq!(error.to_string(), "The file is empty.");

        let too_big = Bytes::from(vec![b'a'; (MEBIBYTE + 1) as usize]);
        let error = f
            .files
            .upload(&caller, owner, "a.txt", too_big)
            .await
            .unwrap_err();
        assert_eq!(error.to_string(), "Files must be at most 1 MB.");
        let exact = Bytes::from(vec![b'a'; MEBIBYTE as usize]);
        assert!(f.files.upload(&caller, owner, "a.txt", exact).await.is_ok());
    }

    /// A WAV file of `seconds` of 8 kHz mono PCM16 silence.
    fn clip(seconds: usize) -> Bytes {
        crate::domains::file::wav(&vec![0; seconds * 16_000], 8_000)
    }

    #[tokio::test]
    async fn refuses_chat_audio_that_hides_its_length() {
        let f = fixture(FileLimits::default());
        let caller = actor();
        let chat = Chat::create(caller.user_id, Utc::now());
        ChatRepository::insert(f.store.as_ref(), &chat)
            .await
            .unwrap();
        let owner = FileOwner::Chat(chat.id());
        let too_long = "Audio in a chat can be at most 5 minutes long.";

        // 301 seconds of 8 kHz mono PCM16 whose header claims a byte rate 2^31 - 1.
        let mut fast = clip(301).to_vec();
        fast[28..32].copy_from_slice(&0x7fff_ffff_u32.to_le_bytes());
        let error = f
            .files
            .upload(&caller, owner, "fast.wav", Bytes::from(fast.clone()))
            .await
            .unwrap_err();
        assert_eq!(
            error.to_string(),
            "We could not tell how long this audio plays. Send a PCM WAV or an MP3 file."
        );
        assert!(
            matches!(&error, DomainError::Validation { field: Some(field), .. } if field == "file")
        );

        // The same clip with a second `fmt ` chunk that claims that rate.
        let honest = clip(301);
        let mut twice = honest[..36].to_vec();
        twice.extend_from_slice(&fast[12..36]);
        twice.extend_from_slice(&honest[36..]);
        let error = f
            .files
            .upload(&caller, owner, "twice.wav", Bytes::from(twice))
            .await
            .unwrap_err();
        assert!(
            error.to_string().starts_with("We could not tell"),
            "{error}"
        );

        // A 301-second MP3 whose Xing frame (64 kbps, 192 bytes) claims 10 frames, then 12,541
        // frames at 8 kbps: every frame counts.
        let mut frame = vec![0xff, 0xf3, 0x14, 0xc0];
        frame.resize(24, 0);
        let mut xing = vec![0xff, 0xf3, 0x84, 0xc0];
        xing.resize(4 + 9, 0);
        xing.extend_from_slice(b"Xing\0\0\0\x01");
        xing.extend_from_slice(&10_u32.to_be_bytes());
        xing.resize(192, 0);
        xing.extend_from_slice(&frame.repeat(12_541));
        let error = f
            .files
            .upload(&caller, owner, "xing.mp3", Bytes::from(xing))
            .await
            .unwrap_err();
        assert_eq!(error.to_string(), too_long);

        // An MP3 with a tag and no frame has no length: refused in a chat, kept in a project.
        let tag = Bytes::from_static(b"ID3\x04\0\0\0\0\0\0");
        let error = f
            .files
            .upload(&caller, owner, "tag.mp3", tag.clone())
            .await
            .unwrap_err();
        assert!(
            error.to_string().starts_with("We could not tell"),
            "{error}"
        );
        let project = new_project(&f, &caller).await;
        f.files
            .upload(&caller, FileOwner::Project(project), "tag.mp3", tag)
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn caps_audio_for_a_chat_at_five_minutes() {
        let f = fixture(FileLimits::default());
        let caller = actor();
        let chat = Chat::create(caller.user_id, Utc::now());
        ChatRepository::insert(f.store.as_ref(), &chat)
            .await
            .unwrap();
        let owner = FileOwner::Chat(chat.id());
        let longest = f
            .files
            .upload(&caller, owner, "clip.wav", clip(300))
            .await
            .unwrap();
        assert_eq!(longest.media_type(), "audio/wav");

        let error = f
            .files
            .upload(&caller, owner, "clip.wav", clip(301))
            .await
            .unwrap_err();
        assert_eq!(
            error.to_string(),
            "Audio in a chat can be at most 5 minutes long."
        );
        assert!(
            matches!(&error, DomainError::Validation { field: Some(field), .. } if field == "file")
        );

        // A constant-bitrate MP3 of 301 seconds at 8 kbps (MPEG-2, 24 kHz, mono): 12,542 frames
        // of 24 bytes and 24 milliseconds each.
        let mut frame = vec![0xff, 0xf3, 0x14, 0xc0];
        frame.resize(24, 0);
        let mp3 = frame.repeat(12_542);
        let error = f
            .files
            .upload(&caller, owner, "talk.mp3", Bytes::from(mp3.clone()))
            .await
            .unwrap_err();
        assert_eq!(
            error.to_string(),
            "Audio in a chat can be at most 5 minutes long."
        );

        // Project audio never reaches a model: only the size limit applies.
        let project = new_project(&f, &caller).await;
        f.files
            .upload(&caller, FileOwner::Project(project), "clip.wav", clip(301))
            .await
            .unwrap();
        f.files
            .upload(
                &caller,
                FileOwner::Project(project),
                "talk.mp3",
                Bytes::from(mp3),
            )
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn enforces_the_storage_quota_per_user() {
        let limits = FileLimits {
            max_upload_mb: 1,
            max_storage_mb: 2,
        };
        let f = fixture(limits);
        let alice = actor();
        let owner = FileOwner::Project(new_project(&f, &alice).await);
        let mebibyte = Bytes::from(vec![b'a'; MEBIBYTE as usize]);
        f.files
            .upload(&alice, owner, "1.txt", mebibyte.clone())
            .await
            .unwrap();
        f.files
            .upload(&alice, owner, "2.txt", mebibyte.clone())
            .await
            .unwrap();
        let error = f
            .files
            .upload(&alice, owner, "3.txt", text("x"))
            .await
            .unwrap_err();
        assert_eq!(error.code(), "STORAGE_FULL");
        assert_eq!(
            error.to_string(),
            "Your file storage is full (2 MB). Delete files to make room."
        );

        let bob = actor();
        let bob_owner = FileOwner::Project(new_project(&f, &bob).await);
        assert!(
            f.files
                .upload(&bob, bob_owner, "1.txt", mebibyte)
                .await
                .is_ok()
        );
    }

    /// The store's files, with `usage` read before other uploads at once were stored (it always
    /// reports an empty storage).
    struct Racing(Arc<MemoryStore>);

    #[async_trait::async_trait]
    impl FileRepository for Racing {
        async fn list(&self, user: UserId, owner: FileOwner) -> DomainResult<Vec<StoredFile>> {
            FileRepository::list(self.0.as_ref(), user, owner).await
        }

        async fn get(&self, user: UserId, id: FileId) -> DomainResult<Option<StoredFile>> {
            FileRepository::get(self.0.as_ref(), user, id).await
        }

        async fn create(&self, file: &StoredFile, bytes: Bytes, quota: u64) -> DomainResult<bool> {
            FileRepository::create(self.0.as_ref(), file, bytes, quota).await
        }

        async fn content(&self, user: UserId, id: FileId) -> DomainResult<Option<Bytes>> {
            FileRepository::content(self.0.as_ref(), user, id).await
        }

        async fn update(&self, file: &StoredFile) -> DomainResult<()> {
            FileRepository::update(self.0.as_ref(), file).await
        }

        async fn move_to(
            &self,
            user: UserId,
            ids: &[FileId],
            project: ProjectId,
        ) -> DomainResult<()> {
            FileRepository::move_to(self.0.as_ref(), user, ids, project).await
        }

        async fn delete(&self, user: UserId, id: FileId) -> DomainResult<bool> {
            FileRepository::delete(self.0.as_ref(), user, id).await
        }

        async fn usage(&self, _user: UserId) -> DomainResult<u64> {
            Ok(0)
        }
    }

    #[tokio::test]
    async fn uploads_at_once_never_pass_the_quota_together() {
        let clock = Arc::new(FixedClock::default());
        let store = Arc::new(MemoryStore::new(clock.clone()));
        let limits = FileLimits {
            max_upload_mb: 1,
            max_storage_mb: 2,
        };
        let files = FileService::new(
            Arc::new(Racing(store.clone())),
            store.clone(),
            store.clone(),
            store.clone(),
            clock.clone(),
            limits,
        );
        let projects = ProjectService::new(store.clone(), store.clone(), store.clone(), clock);
        let alice = actor();
        let input = NewProject {
            name: "Docs".to_owned(),
            ..NewProject::default()
        };
        let owner = FileOwner::Project(projects.create(&alice, input).await.unwrap().id());
        let mebibyte = Bytes::from(vec![b'a'; MEBIBYTE as usize]);
        for name in ["1.txt", "2.txt"] {
            files
                .upload(&alice, owner, name, mebibyte.clone())
                .await
                .unwrap();
        }
        let error = files
            .upload(&alice, owner, "3.txt", mebibyte)
            .await
            .unwrap_err();
        assert_eq!(error.code(), "STORAGE_FULL", "the store's own check");
        assert_eq!(
            FileRepository::usage(store.as_ref(), alice.user_id)
                .await
                .unwrap(),
            2 * MEBIBYTE
        );
    }

    #[tokio::test]
    async fn rejects_another_users_project_and_fails_closed_for_chats() {
        let f = fixture(FileLimits::default());
        let alice = actor();
        let bob = actor();
        let project = new_project(&f, &alice).await;
        let error = f
            .files
            .upload(&bob, FileOwner::Project(project), "a.txt", text("x"))
            .await
            .unwrap_err();
        assert!(is_not_found(&error, "Project"));
        let error = f.files.list(&bob, project).await.unwrap_err();
        assert!(is_not_found(&error, "Project"));

        let error = f
            .files
            .upload(
                &alice,
                FileOwner::Chat(ChatId::generate()),
                "a.txt",
                text("x"),
            )
            .await
            .unwrap_err();
        assert!(is_not_found(&error, "Chat"));
    }

    #[tokio::test]
    async fn hides_other_users_files() {
        let f = fixture(FileLimits::default());
        let alice = actor();
        let bob = actor();
        let project = new_project(&f, &alice).await;
        let bob_project = new_project(&f, &bob).await;
        let file = f
            .files
            .upload(&alice, FileOwner::Project(project), "a.txt", text("x"))
            .await
            .unwrap();
        for error in [
            f.files.get(&bob, file.id()).await.unwrap_err(),
            f.files.content(&bob, file.id()).await.unwrap_err(),
            f.files
                .update(&bob, file.id(), FileUpdate::default())
                .await
                .unwrap_err(),
            f.files
                .move_to_project(&bob, file.id(), bob_project)
                .await
                .unwrap_err(),
            f.files.delete(&bob, file.id()).await.unwrap_err(),
        ] {
            assert!(is_not_found(&error, "File"), "{error}");
        }
        let error = f
            .files
            .move_to_project(&alice, file.id(), bob_project)
            .await
            .unwrap_err();
        assert!(
            is_not_found(&error, "Project"),
            "a move into another user's project"
        );
    }

    #[tokio::test]
    async fn renames_and_excludes_project_files() {
        let f = fixture(FileLimits::default());
        let caller = actor();
        let project = new_project(&f, &caller).await;
        let file = f
            .files
            .upload(&caller, FileOwner::Project(project), "a.txt", text("x"))
            .await
            .unwrap();
        let changes = FileUpdate {
            name: Some("notes/b.txt".to_owned()),
            included: Some(false),
        };
        let updated = f.files.update(&caller, file.id(), changes).await.unwrap();
        assert_eq!(updated.name(), "b.txt");
        assert!(!updated.included());
        assert_eq!(
            updated.media_type(),
            "text/plain",
            "a rename never changes the type"
        );
        assert_eq!(f.files.get(&caller, file.id()).await.unwrap(), updated);
    }

    #[tokio::test]
    async fn moves_files_between_projects() {
        let f = fixture(FileLimits::default());
        let caller = actor();
        let first = new_project(&f, &caller).await;
        let second = new_project(&f, &caller).await;
        let file = f
            .files
            .upload(&caller, FileOwner::Project(first), "a.txt", text("x"))
            .await
            .unwrap();
        let moved = f
            .files
            .move_to_project(&caller, file.id(), second)
            .await
            .unwrap();
        assert_eq!(moved.owner(), FileOwner::Project(second));
        assert_eq!(f.files.get(&caller, file.id()).await.unwrap(), moved);
        assert!(f.files.list(&caller, first).await.unwrap().is_empty());
        let again = f
            .files
            .move_to_project(&caller, file.id(), second)
            .await
            .unwrap();
        assert_eq!(again, moved, "moving into the same project changes nothing");
    }

    #[tokio::test]
    async fn deletes_files_and_frees_their_storage() {
        let limits = FileLimits {
            max_upload_mb: 1,
            max_storage_mb: 1,
        };
        let f = fixture(limits);
        let caller = actor();
        let owner = FileOwner::Project(new_project(&f, &caller).await);
        let mebibyte = Bytes::from(vec![b'a'; MEBIBYTE as usize]);
        let file = f
            .files
            .upload(&caller, owner, "1.txt", mebibyte.clone())
            .await
            .unwrap();
        assert!(
            f.files
                .upload(&caller, owner, "2.txt", text("x"))
                .await
                .is_err()
        );
        f.files.delete(&caller, file.id()).await.unwrap();
        assert!(
            f.files
                .upload(&caller, owner, "2.txt", mebibyte)
                .await
                .is_ok()
        );
        let error = f.files.delete(&caller, file.id()).await.unwrap_err();
        assert!(is_not_found(&error, "File"));
    }

    #[tokio::test]
    async fn deleting_a_project_deletes_its_files() {
        let f = fixture(FileLimits::default());
        let caller = actor();
        let project = new_project(&f, &caller).await;
        let file = f
            .files
            .upload(&caller, FileOwner::Project(project), "a.txt", text("x"))
            .await
            .unwrap();
        f.projects.delete(&caller, project).await.unwrap();
        let error = f.files.get(&caller, file.id()).await.unwrap_err();
        assert!(is_not_found(&error, "File"));
    }

    #[tokio::test]
    async fn measures_capacity_against_the_default_model() {
        let f = fixture(FileLimits::default());
        let caller = actor();
        let project = new_project(&f, &caller).await;
        let owner = FileOwner::Project(project);
        f.files
            .upload(&caller, owner, "a.png", Bytes::from_static(PNG))
            .await
            .unwrap();
        let excluded = f
            .files
            .upload(&caller, owner, "b.png", Bytes::from_static(PNG))
            .await
            .unwrap();
        let changes = FileUpdate {
            included: Some(false),
            ..FileUpdate::default()
        };
        f.files
            .update(&caller, excluded.id(), changes)
            .await
            .unwrap();

        let capacity = f.files.capacity(&caller, project).await.unwrap();
        assert_eq!(capacity.used, 1_600);
        assert_eq!(
            capacity.level(),
            CapacityLevel::Unknown,
            "no default model yet"
        );

        let provider = f
            .providers
            .create(
                &caller,
                NewProvider {
                    kind: ProviderKind::OpenAi,
                    preset: Preset::OpenAi,
                    name: "OpenAI".to_owned(),
                    base_url: None,
                    api_key: None,
                },
            )
            .await
            .unwrap();
        f.gateway.push_models(vec![remote_model("m")]);
        f.providers.refresh(&caller, provider.id()).await.unwrap();
        let model = f
            .models
            .list(&caller, ModelQuery::provider(provider.id()))
            .await
            .unwrap()[0]
            .model
            .id();
        let window = ModelUpdate {
            context_window: Some(Some(1_900)),
            ..ModelUpdate::default()
        };
        f.models.update(&caller, model, window).await.unwrap();
        let default_model = ProjectUpdate {
            default_model_id: Some(Some(model)),
            ..ProjectUpdate::default()
        };
        f.projects
            .update(&caller, project, default_model)
            .await
            .unwrap();

        let capacity = f.files.capacity(&caller, project).await.unwrap();
        assert_eq!(capacity.window, Some(1_900));
        assert_eq!(capacity.level(), CapacityLevel::Warn);

        let error = f.files.capacity(&actor(), project).await.unwrap_err();
        assert!(is_not_found(&error, "Project"));
    }
}
