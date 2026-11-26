use copilot_core::{
    CapabilityOverrides, ModelQuery, ModelRepository, ProviderRepository, SyncReport,
};
use copilot_database::DbPool;

use crate::{NOW, at, detected, provider, store, user};

#[sqlx::test(migrations = false)]
#[ignore = "requires Postgres: run make test-db"]
async fn connections_round_trip_and_stay_with_their_owner(pool: DbPool) {
    let store = store(pool).await;
    let owner = user(&store).await;
    let other = user(&store).await;
    let saved = provider(&store, owner).await;

    assert_eq!(
        ProviderRepository::get(&store, owner, saved.id())
            .await
            .unwrap(),
        Some(saved.clone())
    );
    assert_eq!(
        ProviderRepository::list(&store, owner).await.unwrap(),
        vec![saved.clone()]
    );
    assert_eq!(
        ProviderRepository::get(&store, other, saved.id())
            .await
            .unwrap(),
        None
    );
    assert!(
        ProviderRepository::list(&store, other)
            .await
            .unwrap()
            .is_empty()
    );

    let changed = saved
        .clone()
        .with_name("Renamed")
        .unwrap()
        .with_key(None)
        .with_enabled(false)
        .touched(at(5));
    ProviderRepository::update(&store, &changed).await.unwrap();
    assert_eq!(
        ProviderRepository::get(&store, owner, saved.id())
            .await
            .unwrap(),
        Some(changed)
    );

    assert!(
        !ProviderRepository::delete(&store, other, saved.id())
            .await
            .unwrap(),
        "another user cannot delete it"
    );
    assert!(
        ProviderRepository::delete(&store, owner, saved.id())
            .await
            .unwrap()
    );
    assert_eq!(
        ProviderRepository::get(&store, owner, saved.id())
            .await
            .unwrap(),
        None
    );
}

#[sqlx::test(migrations = false)]
#[ignore = "requires Postgres: run make test-db"]
async fn a_refresh_adds_updates_and_retires_models(pool: DbPool) {
    let store = store(pool).await;
    let owner = user(&store).await;
    let connection = provider(&store, owner).await;

    let report = store
        .sync(
            owner,
            connection.id(),
            vec![detected("a", Some(100_000)), detected("b", None)],
            true,
        )
        .await
        .unwrap();
    assert_eq!(
        report,
        SyncReport {
            added: 2,
            updated: 0,
            unavailable: 0,
            enabled: 2,
        }
    );
    let models = ModelRepository::list(&store, owner, ModelQuery::provider(connection.id()))
        .await
        .unwrap();
    assert_eq!(models.len(), 2);
    let first = models
        .iter()
        .find(|model| model.remote_id() == "a")
        .unwrap()
        .clone();
    assert_eq!(first.display_name(), "A");
    assert!(first.enabled() && first.available());
    assert_eq!(first.created_at(), at(NOW), "from the store's clock");
    assert_eq!(first.updated_at(), at(NOW));
    assert_eq!(first.capabilities(), &detected("a", None).capabilities);
    assert_eq!(
        ModelRepository::get(&store, owner, first.id())
            .await
            .unwrap(),
        Some(first.clone())
    );

    // The user's settings survive a refresh; reported limits replace stored ones.
    let overrides = CapabilityOverrides {
        vision: Some(true),
        ..CapabilityOverrides::default()
    };
    let edited = first
        .clone()
        .with_display_name("Mine".to_owned())
        .with_overrides(overrides.clone())
        .with_limits(Some(100_000), Some(8_000))
        .with_enabled(false);
    ModelRepository::update(&store, &edited).await.unwrap();
    let report = store
        .sync(
            owner,
            connection.id(),
            vec![detected("a", Some(200_000))],
            true,
        )
        .await
        .unwrap();
    assert_eq!(
        report,
        SyncReport {
            added: 0,
            updated: 1,
            unavailable: 1,
            enabled: 0,
        }
    );
    let refreshed = ModelRepository::get(&store, owner, first.id())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(refreshed.display_name(), "Mine");
    assert_eq!(refreshed.overrides(), &overrides);
    assert!(
        !refreshed.enabled(),
        "a refresh never enables a known model"
    );
    assert_eq!(refreshed.context_window(), Some(200_000), "reported wins");
    assert_eq!(
        refreshed.max_output_tokens(),
        Some(8_000),
        "unreported stays"
    );

    let picker = ModelRepository::list(&store, owner, ModelQuery::picker())
        .await
        .unwrap();
    assert!(picker.is_empty(), "a is off and b is no longer listed");
    let report = store
        .sync(owner, connection.id(), Vec::new(), true)
        .await
        .unwrap();
    assert_eq!(report.unavailable, 1, "only models still available count");
}

#[sqlx::test(migrations = false)]
#[ignore = "requires Postgres: run make test-db"]
async fn the_picker_needs_an_enabled_connection_and_the_owner(pool: DbPool) {
    let store = store(pool).await;
    let owner = user(&store).await;
    let other = user(&store).await;
    let connection = provider(&store, owner).await;
    store
        .sync(owner, connection.id(), vec![detected("a", None)], true)
        .await
        .unwrap();
    assert_eq!(
        ModelRepository::list(&store, owner, ModelQuery::picker())
            .await
            .unwrap()
            .len(),
        1
    );
    assert!(
        ModelRepository::list(&store, other, ModelQuery::picker())
            .await
            .unwrap()
            .is_empty()
    );
    assert!(
        store
            .sync(other, connection.id(), vec![detected("x", None)], true)
            .await
            .is_err(),
        "another user's connection is never synced"
    );

    let off = connection.with_enabled(false).touched(at(9));
    ProviderRepository::update(&store, &off).await.unwrap();
    assert!(
        ModelRepository::list(&store, owner, ModelQuery::picker())
            .await
            .unwrap()
            .is_empty()
    );
}
