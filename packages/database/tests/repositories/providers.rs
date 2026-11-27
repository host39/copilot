use copilot_core::{
    CapabilityOverrides, Chat, ChatRepository, ModelId, ModelQuery, ModelRepository, OpenAiApi,
    ProviderId, ProviderRepository, ProviderUsage, SyncReport, UserId,
};
use copilot_database::operations::migrate::MIGRATOR;
use copilot_database::{DbPool, PgStore};
use uuid::Uuid;

use crate::{KEEP, NOW, at, detected, model, provider, store, user};

/// A chat of `owner` on `model`.
async fn chat_on(store: &PgStore, owner: UserId, model: Option<ModelId>) {
    let chat = Chat::create(owner, at(0)).with_model(model);
    ChatRepository::insert(store, &chat).await.unwrap();
}

/// A project of `owner` whose default model is `model`, and a schedule of `owner` on `model`
/// (their repositories' inserts are not needed here).
async fn project_and_schedule_on(store: &PgStore, owner: UserId, model: ModelId) {
    sqlx::query(
        "INSERT INTO projects (created_at, updated_at, id, user_id, default_model_id, name) \
         VALUES ($1, $1, $2, $3, $4, 'Research')",
    )
    .bind(at(0))
    .bind(Uuid::now_v7())
    .bind(owner.uuid())
    .bind(model.uuid())
    .execute(store.pool())
    .await
    .expect("project inserted");
    sqlx::query(
        "INSERT INTO schedules (created_at, updated_at, id, user_id, model_id, enabled, name, \
         cron, timezone, prompt, created_by) \
         VALUES ($1, $1, $2, $3, $4, false, 'Daily', '0 9 * * *', 'UTC', 'Summarize.', 'user')",
    )
    .bind(at(0))
    .bind(Uuid::now_v7())
    .bind(owner.uuid())
    .bind(model.uuid())
    .execute(store.pool())
    .await
    .expect("schedule inserted");
}

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

    assert_eq!(saved.api(), Some(OpenAiApi::ChatCompletions));
    let changed = saved
        .clone()
        .with_name("Renamed")
        .unwrap()
        .with_api(OpenAiApi::Responses)
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
            KEEP,
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
            KEEP,
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
        .sync(owner, connection.id(), Vec::new(), true, KEEP)
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
        .sync(
            owner,
            connection.id(),
            vec![detected("a", None)],
            true,
            KEEP,
        )
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
            .sync(
                other,
                connection.id(),
                vec![detected("x", None)],
                true,
                KEEP
            )
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

#[sqlx::test(migrations = false)]
#[ignore = "requires Postgres: run make test-db"]
async fn a_refresh_writes_a_long_list_in_a_few_statements(pool: DbPool) {
    let store = store(pool).await;
    let owner = user(&store).await;
    let connection = provider(&store, owner).await;
    let listed: Vec<_> = (0..1_500)
        .map(|index| detected(&format!("m{index}"), (index % 2 == 0).then_some(8_000)))
        .collect();

    let report = store
        .sync(owner, connection.id(), listed.clone(), false, KEEP)
        .await
        .unwrap();
    assert_eq!(report.added, 1_500);
    let report = store
        .sync(owner, connection.id(), listed, false, KEEP)
        .await
        .unwrap();
    assert_eq!(
        report,
        SyncReport {
            added: 0,
            updated: 1_500,
            unavailable: 0,
            enabled: 0,
        }
    );
    let models = ModelRepository::list(&store, owner, ModelQuery::provider(connection.id()))
        .await
        .unwrap();
    assert_eq!(models.len(), 1_500);
    let even = models
        .iter()
        .find(|model| model.remote_id() == "m2")
        .unwrap();
    let odd = models
        .iter()
        .find(|model| model.remote_id() == "m3")
        .unwrap();
    assert_eq!(even.context_window(), Some(8_000));
    assert_eq!(odd.context_window(), None);
    assert_eq!(even.display_name(), "M2");
}

#[sqlx::test(migrations = false)]
#[ignore = "requires Postgres: run make test-db"]
async fn a_refresh_past_what_a_connection_keeps_drops_only_spare_models(pool: DbPool) {
    let store = store(pool).await;
    let owner = user(&store).await;
    let connection = provider(&store, owner).await;
    let listed = |ids: &[&str]| ids.iter().map(|id| detected(id, None)).collect::<Vec<_>>();
    store
        .sync(
            owner,
            connection.id(),
            listed(&["a", "b", "c", "on"]),
            true,
            10,
        )
        .await
        .unwrap();
    let models = ModelRepository::list(&store, owner, ModelQuery::provider(connection.id()))
        .await
        .unwrap();
    for model in &models {
        if model.remote_id() != "on" {
            ModelRepository::update(&store, &model.clone().with_enabled(false))
                .await
                .unwrap();
        }
    }
    let used = models
        .iter()
        .find(|model| model.remote_id() == "a")
        .unwrap();
    let chat = Chat::create(owner, at(0)).with_model(Some(used.id()));
    ChatRepository::insert(&store, &chat).await.unwrap();

    // Five held, two kept: only b and c are unavailable, off, and unused.
    store
        .sync(owner, connection.id(), listed(&["d"]), false, 2)
        .await
        .unwrap();
    let mut left: Vec<String> =
        ModelRepository::list(&store, owner, ModelQuery::provider(connection.id()))
            .await
            .unwrap()
            .iter()
            .map(|model| model.remote_id().to_owned())
            .collect();
    left.sort();
    assert_eq!(left, ["a", "d", "on"]);
}

#[sqlx::test(migrations = false)]
#[ignore = "requires Postgres: run make test-db"]
async fn usage_counts_the_owner_rows_that_name_one_of_the_connection_models(pool: DbPool) {
    let store = store(pool).await;
    let owner = user(&store).await;
    let other = user(&store).await;
    let connection = provider(&store, owner).await;
    store
        .sync(
            owner,
            connection.id(),
            vec![detected("first", None), detected("second", None)],
            true,
            KEEP,
        )
        .await
        .unwrap();
    let models = ModelRepository::list(&store, owner, ModelQuery::provider(connection.id()))
        .await
        .unwrap();
    let (first, second) = (models[0].id(), models[1].id());
    let elsewhere = model(&store, owner).await.id();
    let foreign = model(&store, other).await.id();

    assert_eq!(
        store.usage(owner, connection.id()).await.unwrap(),
        ProviderUsage::default(),
        "nothing uses it yet"
    );
    chat_on(&store, owner, Some(first)).await;
    chat_on(&store, owner, Some(second)).await;
    chat_on(&store, owner, Some(elsewhere)).await;
    chat_on(&store, owner, None).await;
    project_and_schedule_on(&store, owner, second).await;
    project_and_schedule_on(&store, owner, elsewhere).await;
    chat_on(&store, other, Some(foreign)).await;
    project_and_schedule_on(&store, other, foreign).await;

    assert_eq!(
        store.usage(owner, connection.id()).await.unwrap(),
        ProviderUsage {
            chats: 2,
            projects: 1,
            schedules: 1,
        }
    );
    assert_eq!(
        store.usage(other, connection.id()).await.unwrap(),
        ProviderUsage::default(),
        "another user's counts never include the owner's rows"
    );
}

/// The migration that added `providers.api`.
const API_MIGRATION: i64 = 20_261_127_200_831;

/// A connection row as the schema before [`API_MIGRATION`] stored it, with one model whose
/// detected capabilities have `tools` off; their ids.
async fn old_connection(
    store: &PgStore,
    owner: UserId,
    kind: &str,
    preset: &str,
    model: &str,
) -> ProviderId {
    let id = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO providers (created_at, updated_at, id, user_id, kind, preset, name, base_url) \
         VALUES ($1, $1, $2, $3, $4, $5, 'Old', 'https://api.example.com/v1')",
    )
    .bind(at(0))
    .bind(id)
    .bind(owner.uuid())
    .bind(kind)
    .bind(preset)
    .execute(store.pool())
    .await
    .expect("provider inserted");
    sqlx::query(
        "INSERT INTO models (created_at, updated_at, id, user_id, provider_id, enabled, \
         remote_id, display_name, capabilities) VALUES ($1, $1, $2, $3, $4, true, $5, $5, $6)",
    )
    .bind(at(0))
    .bind(Uuid::now_v7())
    .bind(owner.uuid())
    .bind(id)
    .bind(model)
    .bind(serde_json::json!({
        "v": 1, "vision": false, "pdf": true, "tools": false, "audio_in": false,
        "audio_out": false, "reasoning": false, "reasoning_levels": [], "default_reasoning": null,
    }))
    .execute(store.pool())
    .await
    .expect("model inserted");
    ProviderId::new(id)
}

#[sqlx::test(migrations = false)]
#[ignore = "requires Postgres: run make test-db"]
async fn the_api_migration_moves_openai_preset_connections_to_responses(pool: DbPool) {
    let store = store(pool).await;
    let owner = user(&store).await;
    let previous = MIGRATOR
        .iter()
        .filter(|migration| {
            migration.migration_type.is_up_migration() && migration.version < API_MIGRATION
        })
        .map(|migration| migration.version)
        .max()
        .expect("migrations before it");
    MIGRATOR
        .undo(store.pool(), previous)
        .await
        .expect("the down migration reverts it");

    let openai = old_connection(&store, owner, "openai", "openai", "GPT-6.1-sol").await;
    let older = old_connection(&store, owner, "openai", "openai", "gpt-5.6-terra").await;
    let custom = old_connection(&store, owner, "openai", "custom", "gpt-6.1-sol").await;
    let claude = old_connection(&store, owner, "anthropic", "anthropic", "claude").await;
    MIGRATOR.run(store.pool()).await.expect("it applies again");

    let api = |id: ProviderId| {
        let store = &store;
        async move {
            ProviderRepository::get(store, owner, id)
                .await
                .unwrap()
                .unwrap()
                .api()
        }
    };
    assert_eq!(api(openai).await, Some(OpenAiApi::Responses));
    assert_eq!(api(older).await, Some(OpenAiApi::Responses));
    assert_eq!(api(custom).await, Some(OpenAiApi::ChatCompletions));
    assert_eq!(api(claude).await, None);

    let tools = |id: ProviderId| {
        let store = &store;
        async move {
            ModelRepository::list(store, owner, ModelQuery::provider(id))
                .await
                .unwrap()[0]
                .capabilities()
                .tools
        }
    };
    assert!(
        tools(openai).await,
        "the gpt-6 lock no longer applies there"
    );
    assert!(!tools(older).await, "other models keep what was detected");
    assert!(!tools(custom).await, "other presets keep what was detected");

    for (kind, api) in [("anthropic", Some("responses")), ("openai", None)] {
        let refused = sqlx::query(
            "INSERT INTO providers (created_at, updated_at, id, user_id, kind, preset, name, \
             base_url, api) VALUES ($1, $1, $2, $3, $4, 'custom', 'Bad', 'https://x.example', $5)",
        )
        .bind(at(0))
        .bind(Uuid::now_v7())
        .bind(owner.uuid())
        .bind(kind)
        .bind(api)
        .execute(store.pool())
        .await;
        assert!(refused.is_err(), "{kind} with {api:?}");
    }
}
