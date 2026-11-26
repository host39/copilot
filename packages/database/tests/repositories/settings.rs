use copilot_core::{SettingsRepository, UserSettings};
use copilot_database::DbPool;

use crate::{at, store, user};

#[sqlx::test(migrations = false)]
#[ignore = "requires Postgres: run make test-db"]
async fn settings_are_upserted_per_user(pool: DbPool) {
    let store = store(pool).await;
    let owner = user(&store).await;
    let other = user(&store).await;
    assert_eq!(
        store.get(owner).await.unwrap(),
        None,
        "no row reads as none"
    );

    let settings = UserSettings::empty(owner, at(0)).with_instructions("Answer briefly.", at(1));
    store.save(&settings).await.unwrap();
    assert_eq!(store.get(owner).await.unwrap(), Some(settings.clone()));

    let changed = settings.with_instructions("Answer in French.", at(2));
    store.save(&changed).await.unwrap();
    let stored = store.get(owner).await.unwrap().unwrap();
    assert_eq!(stored.instructions(), "Answer in French.");
    assert_eq!(stored.created_at(), at(0), "the first save keeps its time");
    assert_eq!(stored.updated_at(), at(2));
    assert_eq!(
        store.get(other).await.unwrap(),
        None,
        "another user's row is not read"
    );
}
