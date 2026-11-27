//! Files: project files, chat attachments, and their bytes (`FileService`). Uploads are
//! `multipart/form-data` with one part named `file` (`UPLOAD_FIELD`), read in chunks and refused
//! as soon as they pass `MAX_UPLOAD_MB`, and only once the request holds an upload slot
//! (`Slots`, a few per user and in all); a body that arrives slower than the upload pace
//! (`UPLOAD_GRACE_SECS`, then `UPLOAD_MIN_BYTES_PER_SEC`) is refused, so a stalled client gives
//! its slot back. The service then checks the type (extension and magic bytes) and the storage
//! quota. The content route sends the file in pieces (`DOWNLOAD_CHUNK_BYTES`, one query each)
//! under a download slot, with the safety headers of "Files" in `docs/design/copilot.md`: an
//! attachment, never rendered inline, with the exact allowed type.

use std::sync::Arc;
use std::time::Duration;

use axum::body::{Body, Bytes};
use axum::extract::multipart::{Multipart, MultipartError, MultipartRejection};
use axum::extract::{DefaultBodyLimit, State};
use axum::http::header::{
    CACHE_CONTROL, CONTENT_DISPOSITION, CONTENT_LENGTH, CONTENT_SECURITY_POLICY, CONTENT_TYPE,
    X_CONTENT_TYPE_OPTIONS,
};
use axum::http::{HeaderName, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use copilot_contracts::{
    DetailsResponse, ErrorResponse, FileResponse, ListResponse, MoveFileRequest, UPLOAD_FIELD,
    UpdateFileRequest, UploadFileForm,
};
use copilot_core::shared::constants::{
    DOWNLOAD_CHUNK_BYTES, DOWNLOAD_STALL_SECS, UPLOAD_GRACE_SECS, UPLOAD_MIN_BYTES_PER_SEC,
};
use copilot_core::{
    Actor, ChatId, DomainError, FileId, FileOwner, FileService, FileUpdate, ProjectId, StoredFile,
    served_type,
};
use copilot_middleware::{HttpError, JsonBody};
use futures_util::stream;
use tokio::sync::mpsc;
use tokio::time::Instant;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use crate::config::AppState;
use crate::helpers::auth::Caller;
use crate::helpers::slots::Slot;
use crate::routes::path::Id;

const MEBIBYTE: usize = 1024 * 1024;

/// The body limit of the upload routes: the largest `MAX_UPLOAD_MB` the config accepts (1000)
/// plus room for the multipart framing. The configured limit is enforced while the file part is
/// read.
const UPLOAD_BODY_MAX: usize = 1_001 * MEBIBYTE;

/// `Content-Security-Policy` of file bytes: nothing loads, and a document opened anyway runs
/// sandboxed.
const CONTENT_POLICY: &str = "default-src 'none'; sandbox";

const CROSS_ORIGIN_RESOURCE_POLICY: HeaderName =
    HeaderName::from_static("cross-origin-resource-policy");

pub fn router() -> OpenApiRouter<AppState> {
    let uploads = OpenApiRouter::new()
        .routes(routes!(upload_to_project, list))
        .routes(routes!(upload_to_chat))
        .layer(DefaultBodyLimit::max(UPLOAD_BODY_MAX));
    OpenApiRouter::new()
        .merge(uploads)
        .routes(routes!(update, delete))
        .routes(routes!(move_to))
        .routes(routes!(content))
}

/// A file as the API shows it (never its bytes).
pub fn response(file: &StoredFile) -> FileResponse {
    let (project_id, chat_id) = match file.owner() {
        FileOwner::Project(project) => (Some(project.uuid()), None),
        FileOwner::Chat(chat) => (None, Some(chat.uuid())),
    };
    FileResponse {
        id: file.id().uuid(),
        project_id,
        chat_id,
        name: file.name().to_owned(),
        media_type: file.media_type().to_owned(),
        size_bytes: file.size_bytes(),
        token_estimate: file.token_estimate(),
        page_count: file.page_count(),
        included: file.included(),
        created_at: file.created_at(),
    }
}

/// A multipart body that is not one part named `file`.
fn malformed() -> HttpError {
    HttpError::Domain(
        DomainError::validation(format!(
            "Send the file as multipart/form-data in one part named {UPLOAD_FIELD}."
        ))
        .with_field(UPLOAD_FIELD),
    )
}

/// A file part past `max_mb`, with the service's own message.
fn too_large(max_mb: u64) -> HttpError {
    HttpError::Domain(
        DomainError::validation(format!("Files must be at most {max_mb} MB."))
            .with_field(UPLOAD_FIELD)
            .with_context("maxUploadMb", max_mb),
    )
}

/// A multipart read that failed: the body limit (`413`) or a malformed body (`400`).
fn unreadable(error: &MultipartError) -> HttpError {
    if error.status() == StatusCode::PAYLOAD_TOO_LARGE {
        HttpError::Status {
            status: StatusCode::PAYLOAD_TOO_LARGE,
            body: Box::new(ErrorResponse::new(
                "Request body is too large.",
                "PAYLOAD_TOO_LARGE",
            )),
        }
    } else {
        malformed()
    }
}

/// A body that fell behind the upload pace (`408`).
fn too_slow() -> HttpError {
    HttpError::Status {
        status: StatusCode::REQUEST_TIMEOUT,
        body: Box::new(ErrorResponse::new(
            "The upload was too slow. Try again on a faster connection.",
            "UPLOAD_TOO_SLOW",
        )),
    }
}

/// How fast an upload body must arrive: by any moment, `UPLOAD_GRACE_SECS` after the read
/// started plus one second per `UPLOAD_MIN_BYTES_PER_SEC` received. A client that trickles
/// (or stops) falls behind and loses its slot; a slow but steady one keeps going.
#[derive(Clone, Copy, Debug)]
struct Pace {
    started: Instant,
    received: u64,
}

impl Pace {
    fn start() -> Self {
        Self {
            started: Instant::now(),
            received: 0,
        }
    }

    fn deadline(&self) -> Instant {
        let earned =
            Duration::from_secs_f64(self.received as f64 / UPLOAD_MIN_BYTES_PER_SEC as f64);
        self.started + Duration::from_secs(UPLOAD_GRACE_SECS) + earned
    }

    /// `future`, or [`too_slow`] once the body falls behind.
    async fn keep<T>(&self, future: impl Future<Output = T>) -> Result<T, HttpError> {
        tokio::time::timeout_at(self.deadline(), future)
            .await
            .map_err(|_| too_slow())
    }
}

/// Reads the one `file` part: its file name and bytes. Any other part, a second `file` part, a
/// file past `max_mb`, or a body behind `pace` stops the read.
async fn upload(
    multipart: Result<Multipart, MultipartRejection>,
    max_mb: u64,
    mut pace: Pace,
) -> Result<(String, Bytes), HttpError> {
    let mut multipart = multipart.map_err(|_| malformed())?;
    let max_bytes = usize::try_from(max_mb)
        .unwrap_or(usize::MAX)
        .saturating_mul(MEBIBYTE);
    let mut found = None;
    while let Some(mut field) = pace
        .keep(multipart.next_field())
        .await?
        .map_err(|error| unreadable(&error))?
    {
        if field.name() != Some(UPLOAD_FIELD) || found.is_some() {
            return Err(malformed());
        }
        let name = field.file_name().unwrap_or_default().to_owned();
        let mut bytes = Vec::new();
        while let Some(chunk) = pace
            .keep(field.chunk())
            .await?
            .map_err(|error| unreadable(&error))?
        {
            if bytes.len().saturating_add(chunk.len()) > max_bytes {
                return Err(too_large(max_mb));
            }
            bytes.extend_from_slice(&chunk);
            pace.received = pace.received.saturating_add(chunk.len() as u64);
        }
        found = Some((name, Bytes::from(bytes)));
    }
    found.ok_or_else(malformed)
}

async fn store(
    state: &AppState,
    caller: &copilot_core::Actor,
    owner: FileOwner,
    multipart: Result<Multipart, MultipartRejection>,
) -> Result<(StatusCode, axum::Json<DetailsResponse<FileResponse>>), HttpError> {
    // Held until the file is stored: the body is in memory until then.
    let _slot = state.uploads.acquire(caller.user_id).await;
    let max_mb = state.config.files.max_upload_mb;
    let (name, bytes) = upload(multipart, max_mb, Pace::start()).await?;
    let file = state.files.upload(caller, owner, &name, bytes).await?;
    let data = response(&file);
    Ok((StatusCode::CREATED, axum::Json(DetailsResponse { data })))
}

#[utoipa::path(
    post,
    path = "/projects/{id}/files",
    operation_id = "files_upload_to_project",
    tag = "files",
    security(("bearerAuth" = [])),
    params(("id" = Uuid, Path, description = "The project's id")),
    request_body(content = UploadFileForm, content_type = "multipart/form-data"),
    responses(
        (status = CREATED, description = "The stored file", body = DetailsResponse<FileResponse>),
        (status = BAD_REQUEST, description = "Not one file part, too large, or a type that is not supported", body = ErrorResponse),
        (status = NOT_FOUND, description = "The caller has no such project", body = ErrorResponse),
        (status = REQUEST_TIMEOUT, description = "The body arrived slower than the upload pace (`UPLOAD_TOO_SLOW`)", body = ErrorResponse),
        (status = UNPROCESSABLE_ENTITY, description = "The caller's file storage is full (`STORAGE_FULL`)", body = ErrorResponse)
    )
)]
pub async fn upload_to_project(
    State(state): State<AppState>,
    Caller(caller): Caller,
    Id(id): Id,
    multipart: Result<Multipart, MultipartRejection>,
) -> Result<(StatusCode, axum::Json<DetailsResponse<FileResponse>>), HttpError> {
    let owner = FileOwner::Project(ProjectId::new(id));
    store(&state, &caller, owner, multipart).await
}

#[utoipa::path(
    get,
    path = "/projects/{id}/files",
    operation_id = "files_list",
    tag = "files",
    security(("bearerAuth" = [])),
    params(("id" = Uuid, Path, description = "The project's id")),
    responses(
        (status = OK, description = "The project's files, oldest first", body = ListResponse<FileResponse>),
        (status = NOT_FOUND, description = "The caller has no such project", body = ErrorResponse)
    )
)]
pub async fn list(
    State(state): State<AppState>,
    Caller(caller): Caller,
    Id(id): Id,
) -> Result<axum::Json<ListResponse<FileResponse>>, HttpError> {
    let files = state.files.list(&caller, ProjectId::new(id)).await?;
    let items: Vec<FileResponse> = files.iter().map(response).collect();
    Ok(axum::Json(ListResponse {
        total: items.len() as u64,
        items,
    }))
}

#[utoipa::path(
    post,
    path = "/chats/{id}/files",
    operation_id = "files_upload_to_chat",
    tag = "files",
    security(("bearerAuth" = [])),
    params(("id" = Uuid, Path, description = "The chat's id")),
    request_body(content = UploadFileForm, content_type = "multipart/form-data"),
    responses(
        (status = CREATED, description = "The stored attachment", body = DetailsResponse<FileResponse>),
        (status = BAD_REQUEST, description = "Not one file part, too large, a type that is not supported, or audio that plays longer than 5 minutes", body = ErrorResponse),
        (status = NOT_FOUND, description = "The caller has no such chat", body = ErrorResponse),
        (status = REQUEST_TIMEOUT, description = "The body arrived slower than the upload pace (`UPLOAD_TOO_SLOW`)", body = ErrorResponse),
        (status = UNPROCESSABLE_ENTITY, description = "The caller's file storage is full (`STORAGE_FULL`)", body = ErrorResponse)
    )
)]
pub async fn upload_to_chat(
    State(state): State<AppState>,
    Caller(caller): Caller,
    Id(id): Id,
    multipart: Result<Multipart, MultipartRejection>,
) -> Result<(StatusCode, axum::Json<DetailsResponse<FileResponse>>), HttpError> {
    let owner = FileOwner::Chat(ChatId::new(id));
    store(&state, &caller, owner, multipart).await
}

#[utoipa::path(
    patch,
    path = "/files/{id}",
    operation_id = "files_update",
    tag = "files",
    security(("bearerAuth" = [])),
    params(("id" = Uuid, Path, description = "The file's id")),
    request_body = UpdateFileRequest,
    responses(
        (status = OK, description = "The saved file", body = DetailsResponse<FileResponse>),
        (status = BAD_REQUEST, description = "A field is not valid", body = ErrorResponse),
        (status = NOT_FOUND, description = "The caller has no such file", body = ErrorResponse)
    )
)]
pub async fn update(
    State(state): State<AppState>,
    Caller(caller): Caller,
    Id(id): Id,
    JsonBody(body): JsonBody<UpdateFileRequest>,
) -> Result<axum::Json<DetailsResponse<FileResponse>>, HttpError> {
    let changes = FileUpdate {
        name: body.name,
        included: body.included,
    };
    let file = state
        .files
        .update(&caller, FileId::new(id), changes)
        .await?;
    Ok(axum::Json(DetailsResponse {
        data: response(&file),
    }))
}

#[utoipa::path(
    delete,
    path = "/files/{id}",
    operation_id = "files_delete",
    tag = "files",
    security(("bearerAuth" = [])),
    params(("id" = Uuid, Path, description = "The file's id")),
    responses(
        (status = NO_CONTENT, description = "Deleted with its bytes"),
        (status = NOT_FOUND, description = "The caller has no such file", body = ErrorResponse)
    )
)]
pub async fn delete(
    State(state): State<AppState>,
    Caller(caller): Caller,
    Id(id): Id,
) -> Result<StatusCode, HttpError> {
    state.files.delete(&caller, FileId::new(id)).await?;
    Ok(StatusCode::NO_CONTENT)
}

#[utoipa::path(
    post,
    path = "/files/{id}/move",
    operation_id = "files_move_to",
    tag = "files",
    security(("bearerAuth" = [])),
    params(("id" = Uuid, Path, description = "The file's id")),
    request_body = MoveFileRequest,
    responses(
        (status = OK, description = "The file in its new project", body = DetailsResponse<FileResponse>),
        (status = NOT_FOUND, description = "The caller has no such file or project", body = ErrorResponse)
    )
)]
pub async fn move_to(
    State(state): State<AppState>,
    Caller(caller): Caller,
    Id(id): Id,
    JsonBody(body): JsonBody<MoveFileRequest>,
) -> Result<axum::Json<DetailsResponse<FileResponse>>, HttpError> {
    let file = state
        .files
        .move_to_project(&caller, FileId::new(id), ProjectId::new(body.project_id))
        .await?;
    Ok(axum::Json(DetailsResponse {
        data: response(&file),
    }))
}

/// `filename*` of a `Content-Disposition` (RFC 6266 and RFC 8187): UTF-8, with every byte
/// outside the unreserved characters percent-encoded, so no quote, semicolon, or line break
/// reaches the header.
fn disposition(name: &str) -> String {
    let mut encoded = String::with_capacity(name.len());
    for byte in name.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~') {
            encoded.push(char::from(byte));
        } else {
            encoded.push_str(&format!("%{byte:02X}"));
        }
    }
    format!("attachment; filename*=UTF-8''{encoded}")
}

/// The headers every file download carries.
fn headers(file: &StoredFile) -> [(HeaderName, HeaderValue); 6] {
    let disposition = HeaderValue::from_str(&disposition(file.name()))
        .unwrap_or_else(|_| HeaderValue::from_static("attachment"));
    [
        (
            CONTENT_TYPE,
            HeaderValue::from_static(served_type(file.media_type())),
        ),
        (CONTENT_DISPOSITION, disposition),
        (X_CONTENT_TYPE_OPTIONS, HeaderValue::from_static("nosniff")),
        (
            CONTENT_SECURITY_POLICY,
            HeaderValue::from_static(CONTENT_POLICY),
        ),
        (
            CROSS_ORIGIN_RESOURCE_POLICY,
            HeaderValue::from_static("same-origin"),
        ),
        (CACHE_CONTROL, HeaderValue::from_static("private, no-store")),
    ]
}

#[utoipa::path(
    get,
    path = "/files/{id}/content",
    operation_id = "files_content",
    tag = "files",
    security(("bearerAuth" = [])),
    params(("id" = Uuid, Path, description = "The file's id")),
    responses(
        (status = OK, description = "The file's bytes, as an attachment with its allowed type (otherwise `application/octet-stream`), `nosniff`, and a sandboxing policy", content_type = "application/octet-stream"),
        (status = NOT_FOUND, description = "The caller has no such file", body = ErrorResponse)
    )
)]
pub async fn content(
    State(state): State<AppState>,
    Caller(caller): Caller,
    Id(id): Id,
) -> Result<Response, HttpError> {
    let file = state.files.get(&caller, FileId::new(id)).await?;
    let slot = state.downloads.acquire(caller.user_id).await;
    let body = pieces(state.files.clone(), caller, &file, slot);
    let mut response = (headers(&file), body).into_response();
    response
        .headers_mut()
        .insert(CONTENT_LENGTH, HeaderValue::from(file.size_bytes()));
    Ok(response)
}

/// The file's bytes as a body read in pieces of `DOWNLOAD_CHUNK_BYTES`, one query each, so a
/// download holds a few pieces at a time and no pool connection between them. A task reads the
/// pieces and holds `slot`; it stops when the body is sent, when the client goes away, or when
/// the client takes no piece for `DOWNLOAD_STALL_SECS`. A failed read ends the body early, so
/// the client sees a short download.
fn pieces(files: Arc<FileService>, caller: Actor, file: &StoredFile, slot: Slot) -> Body {
    let (sender, receiver) = mpsc::channel::<Result<Bytes, DomainError>>(1);
    let (id, size) = (file.id(), file.size_bytes());
    tokio::spawn(async move {
        let _slot = slot;
        let stall = Duration::from_secs(DOWNLOAD_STALL_SECS);
        let mut offset = 0;
        while offset < size {
            let piece = files.piece(&caller, id, offset, DOWNLOAD_CHUNK_BYTES).await;
            let piece = match piece {
                Ok(piece) if piece.is_empty() => {
                    Err(DomainError::not_found("File", id.to_string()))
                }
                other => other,
            };
            match &piece {
                Ok(piece) => offset += piece.len() as u64,
                Err(error) => {
                    tracing::warn!(error = %error, file_id = %id, offset, "api.files.download.failed")
                }
            }
            let failed = piece.is_err();
            match tokio::time::timeout(stall, sender.send(piece)).await {
                Ok(Ok(())) if !failed => {}
                Ok(_) => return,
                Err(_) => {
                    tracing::info!(file_id = %id, offset, "api.files.download.stalled");
                    return;
                }
            }
        }
    });
    Body::from_stream(stream::unfold(receiver, |mut receiver| async move {
        let piece = receiver.recv().await?;
        Some((piece, receiver))
    }))
}

#[cfg(test)]
mod tests {
    use axum::Router;
    use axum::body::Body;
    use axum::extract::FromRequest;
    use axum::http::header::CONTENT_TYPE;
    use axum::http::{Method, Request, StatusCode};
    use copilot_core::{Clock, FileId, ProjectId, StoredFileParts, SystemClock, UserId};
    use copilot_database::DbPool;
    use futures_util::StreamExt;
    use serde_json::{Value, json};
    use uuid::Uuid;

    use super::*;
    use crate::helpers::testing::{
        TestResponse, config, live_app, live_app_with, multipart, other_user, read, register,
        request, send,
    };

    fn stored(name: &str, media_type: &str) -> StoredFile {
        StoredFile::restore(StoredFileParts {
            id: FileId::generate(),
            user_id: UserId::new(Uuid::now_v7()),
            owner: FileOwner::Project(ProjectId::generate()),
            name: name.to_owned(),
            media_type: media_type.to_owned(),
            size_bytes: 4,
            sha256: [0; 32],
            token_estimate: 1,
            page_count: None,
            included: true,
            created_at: SystemClock.now(),
        })
    }

    fn header(file: &StoredFile, name: &HeaderName) -> String {
        headers(file)
            .into_iter()
            .find(|(header, _)| header == name)
            .map(|(_, value)| value.to_str().expect("ascii").to_owned())
            .expect("the header is sent")
    }

    #[test]
    fn file_names_are_percent_encoded_in_the_disposition() {
        assert_eq!(
            disposition("notes.md"),
            "attachment; filename*=UTF-8''notes.md"
        );
        assert_eq!(
            disposition("Q3 report (final).pdf"),
            "attachment; filename*=UTF-8''Q3%20report%20%28final%29.pdf"
        );
        assert_eq!(
            disposition("résumé.md"),
            "attachment; filename*=UTF-8''r%C3%A9sum%C3%A9.md"
        );
        assert_eq!(
            disposition("a\";b=c.txt"),
            "attachment; filename*=UTF-8''a%22%3Bb%3Dc.txt",
            "no quote or semicolon reaches the header"
        );
    }

    #[test]
    fn downloads_are_attachments_with_the_allowed_type_only() {
        let png = stored("chart.png", "image/png");
        assert_eq!(header(&png, &CONTENT_TYPE), "image/png");
        assert_eq!(
            header(&png, &CONTENT_DISPOSITION),
            "attachment; filename*=UTF-8''chart.png"
        );
        assert_eq!(header(&png, &X_CONTENT_TYPE_OPTIONS), "nosniff");
        assert_eq!(
            header(&png, &CONTENT_SECURITY_POLICY),
            "default-src 'none'; sandbox"
        );
        assert_eq!(header(&png, &CROSS_ORIGIN_RESOURCE_POLICY), "same-origin");
        for (name, media_type) in [
            ("page.html", "text/html"),
            ("logo.svg", "image/svg+xml"),
            ("odd.bin", ""),
        ] {
            assert_eq!(
                header(&stored(name, media_type), &CONTENT_TYPE),
                "application/octet-stream",
                "{media_type} is never served as itself"
            );
        }
        assert_eq!(
            header(&stored("page.html", "text/plain"), &CONTENT_TYPE),
            "text/plain",
            "HTML uploads are stored and served as plain text"
        );
    }

    /// Runs [`upload`] on a multipart request with `parts` and a limit of `max_mb`.
    async fn read_upload(
        parts: &[(&str, &str, &[u8])],
        max_mb: u64,
    ) -> Result<(String, axum::body::Bytes), HttpError> {
        let request = multipart("/", "token", parts);
        upload(
            Multipart::from_request(request, &()).await,
            max_mb,
            Pace::start(),
        )
        .await
    }

    /// The message and field of a validation error.
    fn validation(
        result: Result<(String, axum::body::Bytes), HttpError>,
    ) -> (String, Option<String>) {
        match result {
            Err(HttpError::Domain(DomainError::Validation { message, field, .. })) => {
                (message, field)
            }
            Err(other) => panic!("expected a validation error, got {other:?}"),
            Ok((name, _)) => panic!("expected a validation error, read {name}"),
        }
    }

    #[tokio::test]
    async fn an_upload_is_one_file_part_within_the_limit() {
        let (name, bytes) = read_upload(&[("file", "notes.md", b"# Notes")], 1)
            .await
            .expect("one file part");
        assert_eq!(name, "notes.md");
        assert_eq!(bytes.as_ref(), b"# Notes");

        let large = vec![b'a'; MEBIBYTE + 1];
        let (message, field) = validation(read_upload(&[("file", "big.txt", &large)], 1).await);
        assert_eq!(message, "Files must be at most 1 MB.");
        assert_eq!(field.as_deref(), Some("file"));

        for parts in [
            vec![("other", "notes.md", b"# Notes".as_slice())],
            vec![("file", "a.md", b"a".as_slice()), ("file", "b.md", b"b")],
            Vec::new(),
        ] {
            let (_, field) = validation(read_upload(&parts, 1).await);
            assert_eq!(field.as_deref(), Some("file"));
        }

        let json = Request::builder()
            .header(CONTENT_TYPE, "application/json")
            .body(Body::from("{}"))
            .expect("request");
        let multipart = Multipart::from_request(json, &()).await;
        let (_, field) = validation(upload(multipart, 1, Pace::start()).await);
        assert_eq!(field.as_deref(), Some("file"), "not multipart at all");
    }

    #[tokio::test]
    async fn a_body_past_the_route_limit_is_too_large() {
        use tower::ServiceExt;

        let app = Router::new()
            .route(
                "/",
                axum::routing::post(
                    |multipart: Result<Multipart, MultipartRejection>| async move {
                        upload(multipart, 25, Pace::start())
                            .await
                            .map(|_| StatusCode::CREATED)
                    },
                ),
            )
            .layer(DefaultBodyLimit::max(256));
        let response = app
            .oneshot(multipart("/", "token", &[("file", "a.txt", &[b'a'; 1024])]))
            .await
            .expect("infallible router");
        assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE);
    }

    #[test]
    fn the_pace_grants_time_for_what_arrived() {
        let mut pace = Pace::start();
        let grace = Duration::from_secs(UPLOAD_GRACE_SECS);
        assert_eq!(pace.deadline() - pace.started, grace);
        pace.received = UPLOAD_MIN_BYTES_PER_SEC * 10;
        assert_eq!(
            pace.deadline() - pace.started,
            grace + Duration::from_secs(10)
        );
    }

    /// A multipart body that sends its headers and then `byte` once per `every`, forever.
    fn trickle(every: Duration) -> Request<Body> {
        let head = Bytes::from(
            "--b\r\nContent-Disposition: form-data; name=\"file\"; filename=\"a.txt\"\r\n\r\n",
        );
        let tail = stream::unfold(true, move |first| async move {
            if !first {
                tokio::time::sleep(every).await;
            }
            Some((Ok::<_, std::io::Error>(Bytes::from_static(b"a")), false))
        });
        let body = stream::once(async move { Ok(head) }).chain(tail);
        Request::builder()
            .header(CONTENT_TYPE, "multipart/form-data; boundary=b")
            .body(Body::from_stream(body))
            .expect("request")
    }

    #[tokio::test(start_paused = true)]
    async fn a_body_that_falls_behind_the_pace_is_refused() {
        let started = Instant::now();
        let request = trickle(Duration::from_secs(1));
        let result = upload(
            Multipart::from_request(request, &()).await,
            25,
            Pace::start(),
        )
        .await;
        match result {
            Err(HttpError::Status { status, body }) => {
                assert_eq!(status, StatusCode::REQUEST_TIMEOUT);
                assert_eq!(body.code.as_deref(), Some("UPLOAD_TOO_SLOW"));
            }
            other => panic!("expected a timeout, got {:?}", other.map(|(name, _)| name)),
        }
        let took = started.elapsed();
        assert!(
            took >= Duration::from_secs(UPLOAD_GRACE_SECS)
                && took < Duration::from_secs(UPLOAD_GRACE_SECS + 2),
            "one byte a second earns almost nothing past the grace period: {took:?}"
        );
    }

    async fn call(
        app: &Router,
        token: &str,
        method: Method,
        uri: &str,
        body: Option<Value>,
    ) -> TestResponse {
        send(app.clone(), request(method, uri, Some(token), body)).await
    }

    async fn upload_to(
        app: &Router,
        token: &str,
        uri: &str,
        name: &str,
        bytes: &[u8],
    ) -> TestResponse {
        send(app.clone(), multipart(uri, token, &[("file", name, bytes)])).await
    }

    async fn project(app: &Router, token: &str, name: &str) -> String {
        let created = call(
            app,
            token,
            Method::POST,
            "/api/projects",
            Some(json!({ "name": name })),
        )
        .await;
        assert_eq!(created.status, StatusCode::CREATED, "{}", created.body);
        created.body["data"]["id"]
            .as_str()
            .expect("an id")
            .to_owned()
    }

    async fn chat(app: &Router, token: &str) -> String {
        let created = call(app, token, Method::POST, "/api/chats", Some(json!({}))).await;
        assert_eq!(created.status, StatusCode::CREATED, "{}", created.body);
        created.body["data"]["id"]
            .as_str()
            .expect("an id")
            .to_owned()
    }

    #[sqlx::test(migrations = false)]
    #[ignore = "requires Postgres: run make test-db"]
    async fn a_large_file_downloads_whole_in_pieces(pool: DbPool) {
        let app = live_app(pool).await;
        let alice = register(&app, "alice").await;
        let research = project(&app, &alice, "Research").await;
        let length = 2 * DOWNLOAD_CHUNK_BYTES as usize + 1_234;
        let text: Vec<u8> = (0..length).map(|index| b'a' + (index % 26) as u8).collect();
        let uploaded = upload_to(
            &app,
            &alice,
            &format!("/api/projects/{research}/files"),
            "big.txt",
            &text,
        )
        .await;
        assert_eq!(uploaded.status, StatusCode::CREATED, "{}", uploaded.body);
        let file = uploaded.body["data"]["id"]
            .as_str()
            .expect("an id")
            .to_owned();
        let uri = format!("/api/files/{file}/content");

        let (status, headers, body) =
            read(app.clone(), request(Method::GET, &uri, Some(&alice), None)).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(headers[CONTENT_LENGTH], length.to_string().as_str());
        assert_eq!(body.as_bytes(), text.as_slice(), "every piece, in order");

        let bob = other_user(&app, &alice, "bob").await;
        let (status, _, _) = read(app.clone(), request(Method::GET, &uri, Some(&bob), None)).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "another user's file");
    }

    #[sqlx::test(migrations = false)]
    #[ignore = "requires Postgres: run make test-db"]
    async fn files_are_uploaded_read_changed_moved_and_deleted(pool: DbPool) {
        let app = live_app(pool).await;
        let alice = register(&app, "alice").await;
        let research = project(&app, &alice, "Research").await;
        let files_uri = format!("/api/projects/{research}/files");

        let uploaded = upload_to(
            &app,
            &alice,
            &files_uri,
            "../secret/Notes 1.md",
            b"# Notes\n",
        )
        .await;
        assert_eq!(uploaded.status, StatusCode::CREATED, "{}", uploaded.body);
        let data = &uploaded.body["data"];
        assert_eq!(
            data["name"], "Notes 1.md",
            "the display name is the last segment"
        );
        assert_eq!(data["mediaType"], "text/markdown");
        assert_eq!(data["sizeBytes"], 8);
        assert_eq!(data["projectId"], research.as_str());
        assert_eq!(data["chatId"], Value::Null);
        assert_eq!(data["included"], true);
        let file = data["id"].as_str().expect("an id").to_owned();

        let listed = call(&app, &alice, Method::GET, &files_uri, None).await;
        assert_eq!(listed.body["total"], 1);
        assert_eq!(listed.body["items"][0]["id"], file.as_str());
        let details = call(
            &app,
            &alice,
            Method::GET,
            &format!("/api/projects/{research}"),
            None,
        )
        .await;
        assert!(details.body["data"]["capacity"]["used"].as_u64().unwrap() > 0);

        let (status, headers, body) = read(
            app.clone(),
            request(
                Method::GET,
                &format!("/api/files/{file}/content"),
                Some(&alice),
                None,
            ),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body, "# Notes\n");
        assert_eq!(headers[CONTENT_TYPE], "text/markdown");
        assert_eq!(
            headers[CONTENT_DISPOSITION],
            "attachment; filename*=UTF-8''Notes%201.md"
        );
        assert_eq!(headers[X_CONTENT_TYPE_OPTIONS], "nosniff");
        assert_eq!(
            headers[CONTENT_SECURITY_POLICY],
            "default-src 'none'; sandbox"
        );
        assert_eq!(headers["cross-origin-resource-policy"], "same-origin");

        let page = upload_to(
            &app,
            &alice,
            &files_uri,
            "page.html",
            b"<script>alert(1)</script>",
        )
        .await;
        assert_eq!(page.status, StatusCode::CREATED, "{}", page.body);
        assert_eq!(page.body["data"]["mediaType"], "text/plain");
        let page = page.body["data"]["id"].as_str().expect("an id").to_owned();
        let (_, headers, _) = read(
            app.clone(),
            request(
                Method::GET,
                &format!("/api/files/{page}/content"),
                Some(&alice),
                None,
            ),
        )
        .await;
        assert_eq!(
            headers[CONTENT_TYPE], "text/plain",
            "HTML never renders as HTML"
        );

        for (name, bytes) in [
            (
                "logo.svg",
                b"<svg xmlns='http://www.w3.org/2000/svg'/>".as_slice(),
            ),
            ("fake.png", b"not a png"),
            ("tool.exe", b"MZ"),
        ] {
            let refused = upload_to(&app, &alice, &files_uri, name, bytes).await;
            assert_eq!(refused.status, StatusCode::BAD_REQUEST, "{name}");
            assert_eq!(
                refused.body["error"], "This file type is not supported.",
                "{name}"
            );
        }

        let renamed = call(
            &app,
            &alice,
            Method::PATCH,
            &format!("/api/files/{file}"),
            Some(json!({ "name": "Renamed.md", "included": false })),
        )
        .await;
        assert_eq!(renamed.status, StatusCode::OK, "{}", renamed.body);
        assert_eq!(renamed.body["data"]["name"], "Renamed.md");
        assert_eq!(renamed.body["data"]["included"], false);

        let conversation = chat(&app, &alice).await;
        let attachment = upload_to(
            &app,
            &alice,
            &format!("/api/chats/{conversation}/files"),
            "data.csv",
            b"a,b\n1,2\n",
        )
        .await;
        assert_eq!(
            attachment.status,
            StatusCode::CREATED,
            "{}",
            attachment.body
        );
        assert_eq!(attachment.body["data"]["chatId"], conversation.as_str());
        let attachment = attachment.body["data"]["id"]
            .as_str()
            .expect("an id")
            .to_owned();
        let excluded = call(
            &app,
            &alice,
            Method::PATCH,
            &format!("/api/files/{attachment}"),
            Some(json!({ "included": false })),
        )
        .await;
        assert_eq!(
            excluded.status,
            StatusCode::BAD_REQUEST,
            "attachments are always sent"
        );
        let moved = call(
            &app,
            &alice,
            Method::POST,
            &format!("/api/files/{attachment}/move"),
            Some(json!({ "projectId": research })),
        )
        .await;
        assert_eq!(moved.status, StatusCode::OK, "{}", moved.body);
        assert_eq!(moved.body["data"]["projectId"], research.as_str());
        assert_eq!(moved.body["data"]["chatId"], Value::Null);
        let listed = call(&app, &alice, Method::GET, &files_uri, None).await;
        assert_eq!(listed.body["total"], 3);

        let deleted = call(
            &app,
            &alice,
            Method::DELETE,
            &format!("/api/files/{file}"),
            None,
        )
        .await;
        assert_eq!(deleted.status, StatusCode::NO_CONTENT);
        let gone = call(
            &app,
            &alice,
            Method::GET,
            &format!("/api/files/{file}/content"),
            None,
        )
        .await;
        assert_eq!(gone.status, StatusCode::NOT_FOUND);
    }

    #[sqlx::test(migrations = false)]
    #[ignore = "requires Postgres: run make test-db"]
    async fn another_user_gets_404_for_files_and_their_owners(pool: DbPool) {
        let app = live_app(pool).await;
        let alice = register(&app, "alice").await;
        let bob = other_user(&app, &alice, "bob").await;
        let research = project(&app, &alice, "Research").await;
        let conversation = chat(&app, &alice).await;
        let file = upload_to(
            &app,
            &alice,
            &format!("/api/projects/{research}/files"),
            "notes.md",
            b"private",
        )
        .await;
        let file = file.body["data"]["id"].as_str().expect("an id").to_owned();

        for (method, uri, body) in [
            (Method::GET, format!("/api/files/{file}/content"), None),
            (
                Method::PATCH,
                format!("/api/files/{file}"),
                Some(json!({ "name": "x.md" })),
            ),
            (Method::DELETE, format!("/api/files/{file}"), None),
            (Method::GET, format!("/api/projects/{research}/files"), None),
        ] {
            let response = call(&app, &bob, method.clone(), &uri, body).await;
            assert_eq!(response.status, StatusCode::NOT_FOUND, "{method} {uri}");
            assert_eq!(response.body["code"], "ENTITY_NOT_FOUND");
        }
        for uri in [
            format!("/api/projects/{research}/files"),
            format!("/api/chats/{conversation}/files"),
        ] {
            let response = upload_to(&app, &bob, &uri, "planted.md", b"# Planted").await;
            assert_eq!(response.status, StatusCode::NOT_FOUND, "{uri}");
        }

        let bobs = project(&app, &bob, "Bob's").await;
        let own = upload_to(
            &app,
            &bob,
            &format!("/api/projects/{bobs}/files"),
            "b.md",
            b"b",
        )
        .await;
        let own = own.body["data"]["id"].as_str().expect("an id").to_owned();
        let into_theirs = call(
            &app,
            &bob,
            Method::POST,
            &format!("/api/files/{own}/move"),
            Some(json!({ "projectId": research })),
        )
        .await;
        assert_eq!(
            into_theirs.status,
            StatusCode::NOT_FOUND,
            "A's project in a body"
        );
        let taken = call(
            &app,
            &bob,
            Method::POST,
            &format!("/api/files/{file}/move"),
            Some(json!({ "projectId": bobs })),
        )
        .await;
        assert_eq!(
            taken.status,
            StatusCode::NOT_FOUND,
            "A's file into B's project"
        );

        let (status, _, body) = read(
            app.clone(),
            request(
                Method::GET,
                &format!("/api/files/{file}/content"),
                Some(&alice),
                None,
            ),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body, "private", "unchanged");
    }

    #[sqlx::test(migrations = false)]
    #[ignore = "requires Postgres: run make test-db"]
    async fn uploads_stop_at_the_file_limit_and_the_storage_quota(pool: DbPool) {
        let mut limits = config();
        limits.files.max_upload_mb = 1;
        limits.files.max_storage_mb = 2;
        let app = live_app_with(pool, limits).await;
        let alice = register(&app, "alice").await;
        let research = project(&app, &alice, "Research").await;
        let uri = format!("/api/projects/{research}/files");

        let large = vec![b'a'; MEBIBYTE + 1];
        let refused = upload_to(&app, &alice, &uri, "large.txt", &large).await;
        assert_eq!(refused.status, StatusCode::BAD_REQUEST);
        assert_eq!(refused.body["error"], "Files must be at most 1 MB.");

        let part = vec![b'a'; 900 * 1024];
        for name in ["one.txt", "two.txt"] {
            let stored = upload_to(&app, &alice, &uri, name, &part).await;
            assert_eq!(stored.status, StatusCode::CREATED, "{}", stored.body);
        }
        let full = upload_to(&app, &alice, &uri, "three.txt", &part).await;
        assert_eq!(full.status, StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(full.body["code"], "STORAGE_FULL");
        let listed = call(&app, &alice, Method::GET, &uri, None).await;
        assert_eq!(listed.body["total"], 2, "nothing stored past the quota");
    }
}
