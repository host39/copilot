use bytes::Bytes;
use copilot_core::{
    Chat, ChatRepository, FileId, FileOwner, FileRepository, Measured, MediaType, Project,
    ProjectRepository, StoredFile, UserId,
};
use copilot_database::{DbPool, PgStore};

use crate::{at, lock_waiters, store, user};

async fn project(store: &PgStore, owner: UserId, name: &str) -> Project {
    let project = Project::create(owner, name, at(0)).unwrap();
    ProjectRepository::insert(store, &project).await.unwrap();
    project
}

/// A text file of `owner` with `bytes`, created `seconds` after the test's start.
fn text_file(owner: UserId, to: FileOwner, name: &str, bytes: &[u8], seconds: i64) -> StoredFile {
    StoredFile::create(
        owner,
        to,
        name,
        Measured {
            media_type: MediaType::Markdown,
            size_bytes: bytes.len() as u64,
            sha256: [7; 32],
            token_estimate: 3,
            page_count: None,
        },
        at(seconds),
    )
    .unwrap()
}

#[sqlx::test(migrations = false)]
#[ignore = "requires Postgres: run make test-db"]
async fn files_round_trip_with_their_bytes_for_their_owner_only(pool: DbPool) {
    let store = store(pool).await;
    let files: &dyn FileRepository = &store;
    let owner = user(&store).await;
    let other = user(&store).await;
    let research = project(&store, owner, "Research").await;
    let to = FileOwner::Project(research.id());
    let second = text_file(owner, to, "b.md", b"second", 2);
    let first = text_file(owner, to, "a.md", b"# first", 1);
    assert!(
        files
            .create(&second, Bytes::from_static(b"second"), u64::MAX)
            .await
            .unwrap()
    );
    assert!(
        files
            .create(&first, Bytes::from_static(b"# first"), u64::MAX)
            .await
            .unwrap()
    );

    assert_eq!(
        files.list(owner, to).await.unwrap(),
        vec![first.clone(), second.clone()],
        "oldest first"
    );
    assert_eq!(
        files.get(owner, first.id()).await.unwrap(),
        Some(first.clone())
    );
    assert_eq!(
        files.content(owner, first.id()).await.unwrap(),
        Some(Bytes::from_static(b"# first"))
    );
    assert_eq!(files.usage(owner).await.unwrap(), 13);
    assert_eq!(files.usage(other).await.unwrap(), 0);
    assert_eq!(files.get(other, first.id()).await.unwrap(), None);
    assert_eq!(files.content(other, first.id()).await.unwrap(), None);
    assert!(files.list(other, to).await.unwrap().is_empty());
    assert_eq!(files.get(owner, FileId::generate()).await.unwrap(), None);

    let renamed = first
        .clone()
        .with_name("renamed.md")
        .unwrap()
        .with_included(false)
        .unwrap();
    files.update(&renamed).await.unwrap();
    assert_eq!(files.get(owner, first.id()).await.unwrap(), Some(renamed));

    assert!(!files.delete(other, first.id()).await.unwrap());
    assert!(files.delete(owner, first.id()).await.unwrap());
    assert_eq!(files.get(owner, first.id()).await.unwrap(), None);
    assert_eq!(files.content(owner, first.id()).await.unwrap(), None);
    assert_eq!(files.usage(owner).await.unwrap(), 6);
    let contents: i64 = sqlx::query_scalar("SELECT count(*) FROM file_contents WHERE file_id = $1")
        .bind(first.id().uuid())
        .fetch_one(store.pool())
        .await
        .unwrap();
    assert_eq!(contents, 0, "the bytes went with the row");
}

#[sqlx::test(migrations = false)]
#[ignore = "requires Postgres: run make test-db"]
async fn files_move_between_owners_of_one_user(pool: DbPool) {
    let store = store(pool).await;
    let files: &dyn FileRepository = &store;
    let owner = user(&store).await;
    let other = user(&store).await;
    let chat = Chat::create(owner, at(0));
    ChatRepository::insert(&store, &chat).await.unwrap();
    let research = project(&store, owner, "Research").await;
    let theirs = project(&store, other, "Theirs").await;
    let attachment = text_file(owner, FileOwner::Chat(chat.id()), "a.md", b"hi", 1);
    assert!(
        files
            .create(&attachment, Bytes::from_static(b"hi"), u64::MAX)
            .await
            .unwrap()
    );
    assert_eq!(
        files.list(owner, FileOwner::Chat(chat.id())).await.unwrap(),
        vec![attachment.clone()]
    );

    files
        .move_to(owner, &[attachment.id()], research.id())
        .await
        .unwrap();
    let moved = files.get(owner, attachment.id()).await.unwrap().unwrap();
    assert_eq!(moved.owner(), FileOwner::Project(research.id()));
    assert!(moved.included());
    assert!(
        files
            .list(owner, FileOwner::Chat(chat.id()))
            .await
            .unwrap()
            .is_empty()
    );

    assert!(
        files
            .move_to(owner, &[attachment.id()], theirs.id())
            .await
            .is_err(),
        "the composite key refuses another user's project"
    );
    files
        .move_to(other, &[attachment.id()], theirs.id())
        .await
        .unwrap();
    assert_eq!(
        files
            .get(owner, attachment.id())
            .await
            .unwrap()
            .unwrap()
            .owner(),
        FileOwner::Project(research.id()),
        "another user's move changes nothing"
    );

    let foreign = text_file(owner, FileOwner::Project(theirs.id()), "x.md", b"x", 2);
    assert!(
        files
            .create(&foreign, Bytes::from_static(b"x"), u64::MAX)
            .await
            .is_err(),
        "a file never lands in another user's project"
    );
    assert_eq!(files.get(owner, foreign.id()).await.unwrap(), None);
    let contents: i64 = sqlx::query_scalar("SELECT count(*) FROM file_contents WHERE file_id = $1")
        .bind(foreign.id().uuid())
        .fetch_one(store.pool())
        .await
        .unwrap();
    assert_eq!(contents, 0, "row and bytes are one transaction");
}

#[sqlx::test(migrations = false)]
#[ignore = "requires Postgres: run make test-db"]
async fn uploads_at_once_never_pass_the_quota_together(pool: DbPool) {
    let store = store(pool).await;
    let files: &dyn FileRepository = &store;
    let owner = user(&store).await;
    let other = user(&store).await;
    let to = FileOwner::Project(project(&store, owner, "Research").await.id());
    let quota = 10;
    assert!(
        files
            .create(
                &text_file(owner, to, "a.md", b"1234", 1),
                Bytes::from_static(b"1234"),
                quota
            )
            .await
            .unwrap()
    );

    // Both uploads read 4 of 10 bytes used before either is stored; only one of them fits.
    let first = text_file(owner, to, "b.md", b"12345", 2);
    let second = text_file(owner, to, "c.md", b"12345", 3);
    let mut holder = store.pool().begin().await.unwrap();
    sqlx::query(
        "SELECT pg_advisory_xact_lock(hashtext('copilot.files.quota'), hashtext($1::text))",
    )
    .bind(owner.uuid())
    .execute(&mut *holder)
    .await
    .unwrap();
    let (stored_first, stored_second, ()) = tokio::join!(
        files.create(&first, Bytes::from_static(b"12345"), quota),
        files.create(&second, Bytes::from_static(b"12345"), quota),
        async {
            lock_waiters(&store, 2).await;
            holder.commit().await.unwrap();
        },
    );
    let stored = [stored_first.unwrap(), stored_second.unwrap()];
    assert_eq!(
        stored.iter().filter(|stored| **stored).count(),
        1,
        "{stored:?}"
    );
    assert_eq!(files.usage(owner).await.unwrap(), 9);
    let refused = if stored[0] { &second } else { &first };
    assert_eq!(files.get(owner, refused.id()).await.unwrap(), None);
    let contents: i64 = sqlx::query_scalar("SELECT count(*) FROM file_contents WHERE file_id = $1")
        .bind(refused.id().uuid())
        .fetch_one(store.pool())
        .await
        .unwrap();
    assert_eq!(contents, 0, "nothing of the refused upload is stored");

    // The quota counts the user's own files only.
    let theirs = FileOwner::Project(project(&store, other, "Theirs").await.id());
    assert!(
        files
            .create(
                &text_file(other, theirs, "d.md", b"12345", 4),
                Bytes::from_static(b"12345"),
                quota
            )
            .await
            .unwrap()
    );
}
