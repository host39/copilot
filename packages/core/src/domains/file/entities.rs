//! Stored files: project files and chat attachments.

use chrono::{DateTime, Utc};

use crate::domains::file::media::MediaType;
use crate::shared::constants::FILE_NAME_MAX_CHARS;
use crate::shared::errors::{DomainError, DomainResult};
use crate::shared::ids::{ChatId, FileId, ProjectId, UserId};

/// Where a file belongs. Exactly one of `project_id` and `chat_id` is set in storage.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum FileOwner {
    /// A project file, sent as context to every chat of the project while included.
    Project(ProjectId),
    /// A chat attachment.
    Chat(ChatId),
}

/// A file's metadata (`files`). Its bytes live in `file_contents` and are read separately.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StoredFile {
    id: FileId,
    user_id: UserId,
    owner: FileOwner,
    name: String,
    media_type: String,
    size_bytes: u64,
    sha256: [u8; 32],
    token_estimate: u32,
    page_count: Option<u32>,
    included: bool,
    created_at: DateTime<Utc>,
}

/// Every field of a stored file, for [`StoredFile::restore`].
#[derive(Clone, Debug)]
pub struct StoredFileParts {
    pub id: FileId,
    pub user_id: UserId,
    pub owner: FileOwner,
    pub name: String,
    pub media_type: String,
    pub size_bytes: u64,
    pub sha256: [u8; 32],
    pub token_estimate: u32,
    pub page_count: Option<u32>,
    pub included: bool,
    pub created_at: DateTime<Utc>,
}

/// What an accepted upload measured, for [`StoredFile::create`].
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Measured {
    pub media_type: MediaType,
    pub size_bytes: u64,
    pub sha256: [u8; 32],
    pub token_estimate: u32,
    pub page_count: Option<u32>,
}

impl StoredFile {
    /// A new file, included in context.
    pub fn create(
        user_id: UserId,
        owner: FileOwner,
        name: &str,
        measured: Measured,
        now: DateTime<Utc>,
    ) -> DomainResult<Self> {
        Ok(Self {
            id: FileId::generate(),
            user_id,
            owner,
            name: file_name(name)?,
            media_type: measured.media_type.as_str().to_owned(),
            size_bytes: measured.size_bytes,
            sha256: measured.sha256,
            token_estimate: measured.token_estimate,
            page_count: measured.page_count,
            included: true,
            created_at: now,
        })
    }

    /// A row read back from storage.
    pub fn restore(parts: StoredFileParts) -> Self {
        Self {
            id: parts.id,
            user_id: parts.user_id,
            owner: parts.owner,
            name: parts.name,
            media_type: parts.media_type,
            size_bytes: parts.size_bytes,
            sha256: parts.sha256,
            token_estimate: parts.token_estimate,
            page_count: parts.page_count,
            included: parts.included,
            created_at: parts.created_at,
        }
    }

    pub fn with_name(mut self, name: &str) -> DomainResult<Self> {
        self.name = file_name(name)?;
        Ok(self)
    }

    /// Only project files can be left out of the context.
    pub fn with_included(mut self, included: bool) -> DomainResult<Self> {
        if !included && matches!(self.owner, FileOwner::Chat(_)) {
            return Err(DomainError::validation(
                "Only project files can be left out of the context.",
            )
            .with_field("included"));
        }
        self.included = included;
        Ok(self)
    }

    /// The same file, now in `project`.
    #[must_use]
    pub fn moved(mut self, project: ProjectId) -> Self {
        self.owner = FileOwner::Project(project);
        self
    }

    pub fn id(&self) -> FileId {
        self.id
    }

    pub fn user_id(&self) -> UserId {
        self.user_id
    }

    pub fn owner(&self) -> FileOwner {
        self.owner
    }

    /// Display only, never a path.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The stored type (on the allowlist for every file Copilot accepted).
    pub fn media_type(&self) -> &str {
        &self.media_type
    }

    /// The stored type as an allowlisted [`MediaType`], if it is one.
    pub fn media(&self) -> Option<MediaType> {
        self.media_type.parse().ok()
    }

    pub fn size_bytes(&self) -> u64 {
        self.size_bytes
    }

    pub fn sha256(&self) -> &[u8; 32] {
        &self.sha256
    }

    pub fn token_estimate(&self) -> u32 {
        self.token_estimate
    }

    /// PDFs only.
    pub fn page_count(&self) -> Option<u32> {
        self.page_count
    }

    pub fn included(&self) -> bool {
        self.included
    }

    pub fn created_at(&self) -> DateTime<Utc> {
        self.created_at
    }
}

/// A display name: the last path segment of what the client sent, without control characters,
/// trimmed, at most `FILE_NAME_MAX_CHARS` characters.
pub fn file_name(raw: &str) -> DomainResult<String> {
    let last = raw.rsplit(['/', '\\']).next().unwrap_or(raw);
    let cleaned: String = last
        .chars()
        .filter(|character| !character.is_control())
        .collect();
    let name = cleaned.trim();
    if name.is_empty() {
        return Err(DomainError::validation("The file needs a name.").with_field("name"));
    }
    if name.chars().count() > FILE_NAME_MAX_CHARS {
        return Err(DomainError::validation(format!(
            "File names must be at most {FILE_NAME_MAX_CHARS} characters."
        ))
        .with_field("name"));
    }
    Ok(name.to_owned())
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone;
    use uuid::Uuid;

    use super::*;

    fn measured() -> Measured {
        Measured {
            media_type: MediaType::Text,
            size_bytes: 8,
            sha256: [7; 32],
            token_estimate: 2,
            page_count: None,
        }
    }

    fn file(owner: FileOwner) -> StoredFile {
        let now = Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap();
        StoredFile::create(
            UserId::new(Uuid::now_v7()),
            owner,
            "notes.txt",
            measured(),
            now,
        )
        .unwrap()
    }

    #[test]
    fn keeps_only_the_last_path_segment_of_the_name() {
        assert_eq!(file_name("../../etc/passwd").unwrap(), "passwd");
        assert_eq!(
            file_name("C:\\Users\\me\\report.pdf").unwrap(),
            "report.pdf"
        );
        assert_eq!(file_name(" a\u{0000}b.txt ").unwrap(), "ab.txt");
    }

    #[test]
    fn rejects_empty_and_long_names() {
        assert_eq!(
            file_name("dir/").unwrap_err().to_string(),
            "The file needs a name."
        );
        assert_eq!(
            file_name("   ").unwrap_err().to_string(),
            "The file needs a name."
        );
        let long = format!("{}.txt", "a".repeat(FILE_NAME_MAX_CHARS));
        assert_eq!(
            file_name(&long).unwrap_err().to_string(),
            "File names must be at most 255 characters."
        );
    }

    #[test]
    fn starts_included_with_the_detected_type() {
        let file = file(FileOwner::Project(ProjectId::generate()));
        assert!(file.included());
        assert_eq!(file.media_type(), "text/plain");
        assert_eq!(file.media(), Some(MediaType::Text));
    }

    #[test]
    fn lets_only_project_files_leave_the_context() {
        let project_file = file(FileOwner::Project(ProjectId::generate()));
        assert!(!project_file.with_included(false).unwrap().included());
        let attachment = file(FileOwner::Chat(ChatId::generate()));
        let error = attachment.clone().with_included(false).unwrap_err();
        assert_eq!(
            error.to_string(),
            "Only project files can be left out of the context."
        );
        assert!(attachment.with_included(true).is_ok());
    }

    #[test]
    fn moves_into_a_project() {
        let project = ProjectId::generate();
        let moved = file(FileOwner::Chat(ChatId::generate())).moved(project);
        assert_eq!(moved.owner(), FileOwner::Project(project));
    }
}
