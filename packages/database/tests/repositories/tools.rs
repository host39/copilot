use std::collections::BTreeMap;
use std::time::Duration;

use copilot_core::{
    ApprovalMode, ApprovedTool, AuthMode, BaseUrl, Chat, ChatRepository, ClientConfig,
    NetworkScope, Project, ProjectRepository, RemoteTool, ServerOverride, SignIn, SignInFlow,
    SignInRepository, Slug, ToolListing, ToolMode, ToolServer, ToolServerRepository,
};
use copilot_database::DbPool;
use serde_json::json;
use uuid::Uuid;

use crate::{at, server, store, user};

#[sqlx::test(migrations = false)]
#[ignore = "requires Postgres: run make test-db"]
async fn tool_servers_round_trip_with_their_cache_and_modes(pool: DbPool) {
    let store = store(pool).await;
    let owner = user(&store).await;
    let other = user(&store).await;
    let url = BaseUrl::parse("https://mcp.example.com/mcp", NetworkScope::Public).unwrap();
    let listing = ToolListing {
        tools: vec![RemoteTool {
            name: "search".to_owned(),
            title: None,
            description: "Search.".to_owned(),
            input_schema: json!({"type": "object"}),
            read_only: true,
        }],
        protocol_version: "2025-11-25".to_owned(),
        ttl: Some(Duration::from_secs(300)),
    };
    let server = ToolServer::create(owner, "Docs", Slug::parse("docs").unwrap(), url, at(0))
        .unwrap()
        .with_headers(Some(vec![9, 9]))
        .with_approval(ApprovalMode::Auto)
        .with_tool_modes(BTreeMap::from([("search".to_owned(), ToolMode::Off)]))
        .with_listing(&listing, true, at(3));
    ToolServerRepository::insert(&store, &server).await.unwrap();
    let bare = ToolServer::create(
        owner,
        "Web",
        Slug::parse("web").unwrap(),
        BaseUrl::parse("https://web.example.com", NetworkScope::Public).unwrap(),
        at(1),
    )
    .unwrap();
    ToolServerRepository::insert(&store, &bare).await.unwrap();

    assert_eq!(
        ToolServerRepository::list(&store, owner).await.unwrap(),
        vec![server.clone(), bare],
        "oldest first"
    );
    assert_eq!(
        ToolServerRepository::get(&store, owner, server.id())
            .await
            .unwrap(),
        Some(server.clone())
    );
    assert_eq!(
        ToolServerRepository::get(&store, other, server.id())
            .await
            .unwrap(),
        None
    );
    assert!(
        ToolServerRepository::list(&store, other)
            .await
            .unwrap()
            .is_empty()
    );
}

/// A listing of one tool named `name`.
fn listing(name: &str, version: &str) -> ToolListing {
    ToolListing {
        tools: vec![RemoteTool {
            name: name.to_owned(),
            title: Some(name.to_uppercase()),
            description: format!("{name} things."),
            input_schema: json!({"type": "object", "properties": {"q": {"type": "string"}}}),
            read_only: false,
        }],
        protocol_version: version.to_owned(),
        ttl: None,
    }
}

#[sqlx::test(migrations = false)]
#[ignore = "requires Postgres: run make test-db"]
async fn updates_write_settings_and_listings_write_the_cache(pool: DbPool) {
    let store = store(pool).await;
    let owner = user(&store).await;
    let other = user(&store).await;
    let server = server(&store, owner, "docs").await;

    assert!(
        !store
            .save_listing(
                other,
                server.id(),
                server.url(),
                &listing("search", "v1"),
                true,
                at(5)
            )
            .await
            .unwrap()
    );
    assert_eq!(
        ToolServerRepository::get(&store, owner, server.id())
            .await
            .unwrap(),
        Some(server.clone()),
        "another user's listing changes nothing"
    );
    assert!(
        store
            .save_listing(
                owner,
                server.id(),
                server.url(),
                &listing("search", "v1"),
                false,
                at(5)
            )
            .await
            .unwrap()
    );
    let listed = ToolServerRepository::get(&store, owner, server.id())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        listed,
        server
            .clone()
            .with_listing(&listing("search", "v1"), false, at(5))
    );

    let url = BaseUrl::parse("https://mcp.example.com/v2", NetworkScope::Public).unwrap();
    let changed = listed
        .clone()
        .with_name("Docs v2")
        .unwrap()
        .with_url(url)
        .with_headers(Some(vec![4, 2]))
        .with_approval(ApprovalMode::Auto)
        .with_tool_modes(BTreeMap::from([("search".to_owned(), ToolMode::Ask)]))
        .with_enabled(false)
        .touched(at(9));
    ToolServerRepository::update(&store, &changed)
        .await
        .unwrap();
    let stored = ToolServerRepository::get(&store, owner, server.id())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(stored, changed, "every setting, and the cache stays");
    assert!(stored.review_required(), "a new URL asks until reviewed");
    assert_eq!(stored.protocol_version(), None);
    assert_eq!(stored.slug().as_str(), "docs");

    let foreign = ToolServer::restore(copilot_core::ToolServerParts {
        id: changed.id(),
        user_id: other,
        name: "Taken".to_owned(),
        slug: changed.slug().clone(),
        url: changed.url().clone(),
        headers_sealed: None,
        auth: copilot_core::AuthMode::None,
        client: None,
        sign_in: None,
        approval: ApprovalMode::Auto,
        tool_modes: BTreeMap::new(),
        enabled: true,
        protocol_version: None,
        cache: None,
        review_required: false,
        created_at: at(0),
        updated_at: at(11),
    });
    ToolServerRepository::update(&store, &foreign)
        .await
        .unwrap();
    assert_eq!(
        ToolServerRepository::get(&store, owner, server.id())
            .await
            .unwrap()
            .unwrap()
            .name(),
        "Docs v2",
        "another user's update changes nothing"
    );

    let reviewed = stored.reviewed().touched(at(10));
    ToolServerRepository::update(&store, &reviewed)
        .await
        .unwrap();
    assert!(
        !ToolServerRepository::get(&store, owner, server.id())
            .await
            .unwrap()
            .unwrap()
            .review_required()
    );
}

#[sqlx::test(migrations = false)]
#[ignore = "requires Postgres: run make test-db"]
async fn a_stale_list_expires_and_keeps_its_tools_and_fetch_time(pool: DbPool) {
    let store = store(pool).await;
    let owner = user(&store).await;
    let other = user(&store).await;
    let server = server(&store, owner, "docs").await;
    // A server that was never listed has nothing to expire.
    store.stale(owner, server.id()).await.unwrap();
    let unlisted = ToolServerRepository::get(&store, owner, server.id())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(unlisted.cache(), None);

    let mut listed = listing("search", "v1");
    listed.ttl = Some(Duration::from_secs(600));
    assert!(
        store
            .save_listing(owner, server.id(), server.url(), &listed, false, at(1))
            .await
            .unwrap()
    );
    store.stale(other, server.id()).await.unwrap();
    let fresh = ToolServerRepository::get(&store, owner, server.id())
        .await
        .unwrap()
        .unwrap();
    assert!(
        !fresh.needs_refresh(at(2)),
        "another user cannot expire the list"
    );

    store.stale(owner, server.id()).await.unwrap();
    let stale = ToolServerRepository::get(&store, owner, server.id())
        .await
        .unwrap()
        .unwrap();
    assert!(stale.needs_refresh(at(2)), "the next turn lists it again");
    assert!(stale.listed(), "the old list stays as the fallback");
    let cache = stale.cache().unwrap();
    assert_eq!(cache.tools, fresh.cache().unwrap().tools);
    assert_eq!(cache.fetched_at, at(1), "the fetch time stays true");
    assert_eq!(cache.ttl, Some(Duration::ZERO));
}

#[sqlx::test(migrations = false)]
#[ignore = "requires Postgres: run make test-db"]
async fn a_listing_of_a_url_changed_meanwhile_is_not_stored(pool: DbPool) {
    let store = store(pool).await;
    let owner = user(&store).await;
    let server = server(&store, owner, "docs").await;
    assert!(
        store
            .save_listing(
                owner,
                server.id(),
                server.url(),
                &listing("search", "v1"),
                false,
                at(1)
            )
            .await
            .unwrap()
    );
    let read = ToolServerRepository::get(&store, owner, server.id())
        .await
        .unwrap()
        .unwrap();
    // A Test (or a turn's refresh) lists the old URL while the user changes it.
    let moved = read
        .clone()
        .with_url(BaseUrl::parse("https://other.example.net/mcp", NetworkScope::Public).unwrap())
        .touched(at(2));
    ToolServerRepository::update(&store, &moved).await.unwrap();
    assert!(
        !store
            .save_listing(
                owner,
                server.id(),
                read.url(),
                &listing("delete", "v2"),
                false,
                at(3)
            )
            .await
            .unwrap()
    );
    let stored = ToolServerRepository::get(&store, owner, server.id())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(stored.url(), moved.url());
    assert_eq!(
        stored.cache(),
        read.cache(),
        "the old URL's list is not stored"
    );
    assert_eq!(stored.protocol_version(), None, "nor its protocol version");
    assert!(!stored.listed(), "the next turn lists the new URL first");
    assert!(stored.review_required());
}

#[sqlx::test(migrations = false)]
#[ignore = "requires Postgres: run make test-db"]
async fn only_a_review_of_the_latest_listing_clears_the_review_flag(pool: DbPool) {
    let store = store(pool).await;
    let owner = user(&store).await;
    let server = server(&store, owner, "docs").await;
    store
        .save_listing(
            owner,
            server.id(),
            server.url(),
            &listing("search", "v1"),
            false,
            at(1),
        )
        .await
        .unwrap();
    let read = ToolServerRepository::get(&store, owner, server.id())
        .await
        .unwrap()
        .unwrap();

    // A turn's refresh finds a changed tool list while the user edits the server.
    store
        .save_listing(
            owner,
            server.id(),
            server.url(),
            &listing("delete", "v2"),
            true,
            at(2),
        )
        .await
        .unwrap();
    ToolServerRepository::update(&store, &read.clone().with_enabled(false).touched(at(3)))
        .await
        .unwrap();
    let stored = ToolServerRepository::get(&store, owner, server.id())
        .await
        .unwrap()
        .unwrap();
    assert!(!stored.enabled());
    assert!(stored.review_required(), "the stale write keeps the flag");
    assert_eq!(stored.protocol_version(), Some("v2"), "and the new version");
    assert_eq!(stored.cache().unwrap().tools[0].name, "delete");

    // A review of what the user saw before that listing does not accept it.
    ToolServerRepository::update(&store, &read.clone().reviewed().touched(at(4)))
        .await
        .unwrap();
    assert!(
        ToolServerRepository::get(&store, owner, server.id())
            .await
            .unwrap()
            .unwrap()
            .review_required()
    );
    // A listing without changes never clears it either.
    store
        .save_listing(
            owner,
            server.id(),
            server.url(),
            &listing("delete", "v2"),
            false,
            at(5),
        )
        .await
        .unwrap();
    let latest = ToolServerRepository::get(&store, owner, server.id())
        .await
        .unwrap()
        .unwrap();
    assert!(latest.review_required());
    ToolServerRepository::update(&store, &latest.reviewed().touched(at(6)))
        .await
        .unwrap();
    assert!(
        !ToolServerRepository::get(&store, owner, server.id())
            .await
            .unwrap()
            .unwrap()
            .review_required(),
        "a review of the latest listing clears it"
    );
}

#[sqlx::test(migrations = false)]
#[ignore = "requires Postgres: run make test-db"]
async fn deleting_a_server_removes_it_from_projects_and_chats(pool: DbPool) {
    let store = store(pool).await;
    let owner = user(&store).await;
    let other = user(&store).await;
    let docs = server(&store, owner, "docs").await;
    let web = server(&store, owner, "web").await;
    let project = Project::create(owner, "Research", at(0))
        .unwrap()
        .with_tool_servers(vec![docs.id(), web.id()]);
    ProjectRepository::insert(&store, &project).await.unwrap();
    let chat = Chat::create(owner, at(0))
        .with_tool_overrides(vec![ServerOverride {
            server_id: docs.id(),
            enabled: false,
        }])
        .approving(ApprovedTool {
            server_id: docs.id(),
            name: "search".to_owned(),
        });
    ChatRepository::insert(&store, &chat).await.unwrap();

    assert!(
        !ToolServerRepository::delete(&store, other, docs.id())
            .await
            .unwrap()
    );
    assert!(
        ToolServerRepository::get(&store, owner, docs.id())
            .await
            .unwrap()
            .is_some()
    );
    assert!(
        ToolServerRepository::delete(&store, owner, docs.id())
            .await
            .unwrap()
    );
    assert!(
        !ToolServerRepository::delete(&store, owner, docs.id())
            .await
            .unwrap(),
        "already gone"
    );
    assert_eq!(
        ToolServerRepository::list(&store, owner).await.unwrap(),
        vec![web.clone()]
    );
    let project = ProjectRepository::get(&store, owner, project.id())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(project.tool_server_ids(), &[web.id()]);
    let chat = ChatRepository::get(&store, owner, chat.id())
        .await
        .unwrap()
        .unwrap();
    assert!(chat.tool_overrides().is_empty());
    assert!(chat.approved_tools().is_empty());
}

/// An OAuth server of `owner` with a pre-registered client and no sign-in.
async fn oauth_server(
    store: &copilot_database::PgStore,
    owner: copilot_core::UserId,
) -> ToolServer {
    let url = BaseUrl::parse("https://mcp.example.com/mcp", NetworkScope::Public).unwrap();
    let server = ToolServer::create(owner, "Docs", Slug::parse("docs").unwrap(), url, at(0))
        .unwrap()
        .with_auth(AuthMode::OAuth)
        .with_client(Some(ClientConfig {
            id: "client-1".to_owned(),
            secret_sealed: Some(vec![7, 7]),
        }));
    ToolServerRepository::insert(store, &server).await.unwrap();
    server
}

fn sign_in(byte: u8, refreshable: bool) -> SignIn {
    SignIn {
        sealed: vec![byte; 4],
        expires_at: Some(at(60)),
        refreshable,
    }
}

async fn stored(store: &copilot_database::PgStore, server: &ToolServer) -> ToolServer {
    ToolServerRepository::get(store, server.user_id(), server.id())
        .await
        .unwrap()
        .unwrap()
}

#[sqlx::test(migrations = false)]
#[ignore = "requires Postgres: run make test-db"]
async fn sign_ins_round_trip_and_replace_only_what_was_read(pool: DbPool) {
    let store = store(pool).await;
    let owner = user(&store).await;
    let other = user(&store).await;
    let server = oauth_server(&store, owner).await;
    assert_eq!(stored(&store, &server).await, server, "mode and client");

    let first = sign_in(1, true);
    assert!(
        ToolServerRepository::save_sign_in(
            &store,
            owner,
            server.id(),
            server.url(),
            Some(&first),
            None
        )
        .await
        .unwrap()
    );
    assert_eq!(stored(&store, &server).await.sign_in(), Some(&first));

    let second = sign_in(2, false);
    let stale = ToolServerRepository::save_sign_in(
        &store,
        owner,
        server.id(),
        server.url(),
        Some(&second),
        Some(&[9, 9, 9, 9]),
    )
    .await
    .unwrap();
    assert!(!stale, "another refresh stored a different sign-in");
    assert!(
        ToolServerRepository::save_sign_in(
            &store,
            owner,
            server.id(),
            server.url(),
            Some(&second),
            Some(&first.sealed),
        )
        .await
        .unwrap()
    );
    assert_eq!(stored(&store, &server).await.sign_in(), Some(&second));

    let foreign =
        ToolServerRepository::save_sign_in(&store, other, server.id(), server.url(), None, None)
            .await
            .unwrap();
    assert!(!foreign, "another user's write changes nothing");
    let elsewhere = BaseUrl::parse("https://other.example.com/mcp", NetworkScope::Public).unwrap();
    assert!(
        !ToolServerRepository::save_sign_in(&store, owner, server.id(), &elsewhere, None, None)
            .await
            .unwrap(),
        "only for the URL the sign-in was for"
    );
    assert!(
        ToolServerRepository::save_sign_in(&store, owner, server.id(), server.url(), None, None)
            .await
            .unwrap()
    );
    assert_eq!(stored(&store, &server).await.sign_in(), None, "signed out");

    let plain = ToolServer::create(
        owner,
        "Web",
        Slug::parse("web").unwrap(),
        BaseUrl::parse("https://web.example.com", NetworkScope::Public).unwrap(),
        at(1),
    )
    .unwrap();
    ToolServerRepository::insert(&store, &plain).await.unwrap();
    assert!(
        !ToolServerRepository::save_sign_in(
            &store,
            owner,
            plain.id(),
            plain.url(),
            Some(&first),
            None
        )
        .await
        .unwrap(),
        "only an OAuth server takes a sign-in"
    );
}

#[sqlx::test(migrations = false)]
#[ignore = "requires Postgres: run make test-db"]
async fn settings_writes_keep_a_sign_in_until_the_mode_or_url_changes(pool: DbPool) {
    let store = store(pool).await;
    let owner = user(&store).await;
    let server = oauth_server(&store, owner).await;
    let first = sign_in(1, true);
    ToolServerRepository::save_sign_in(
        &store,
        owner,
        server.id(),
        server.url(),
        Some(&first),
        None,
    )
    .await
    .unwrap();

    // Read before the sign-in: a settings save must not undo it.
    let renamed = server.clone().with_name("Docs v2").unwrap().touched(at(5));
    ToolServerRepository::update(&store, &renamed)
        .await
        .unwrap();
    let after = stored(&store, &server).await;
    assert_eq!(after.name(), "Docs v2");
    assert_eq!(after.sign_in(), Some(&first));

    let moved = after
        .clone()
        .with_url(BaseUrl::parse("https://mcp.example.com/v2", NetworkScope::Public).unwrap());
    ToolServerRepository::update(&store, &moved).await.unwrap();
    assert_eq!(stored(&store, &server).await.sign_in(), None, "a new URL");

    let moved = stored(&store, &server).await;
    ToolServerRepository::save_sign_in(&store, owner, server.id(), moved.url(), Some(&first), None)
        .await
        .unwrap();
    let headers = moved.with_headers(Some(vec![5]));
    ToolServerRepository::update(&store, &headers)
        .await
        .unwrap();
    let after = stored(&store, &server).await;
    assert_eq!(after.auth(), AuthMode::Headers);
    assert_eq!(after.sign_in(), None, "another mode");
    assert_eq!(after.client(), None);
    assert_eq!(after.headers_sealed(), Some(&[5][..]));
}

#[sqlx::test(migrations = false)]
#[ignore = "requires Postgres: run make test-db"]
async fn the_database_refuses_credentials_of_another_mode(pool: DbPool) {
    let store = store(pool.clone()).await;
    let owner = user(&store).await;
    let server = oauth_server(&store, owner).await;
    let mixed = sqlx::query("UPDATE tool_servers SET headers_sealed = '\\x01' WHERE id = $1")
        .bind(server.id().uuid())
        .execute(&pool)
        .await;
    assert!(mixed.is_err(), "headers in oauth mode");
    let orphan = sqlx::query(
        "UPDATE tool_servers SET auth_mode = 'none', oauth_client_id = NULL, \
         oauth_client_secret_sealed = NULL, oauth_tokens_sealed = '\\x01' WHERE id = $1",
    )
    .bind(server.id().uuid())
    .execute(&pool)
    .await;
    assert!(orphan.is_err(), "tokens outside oauth mode");
}

fn flow(server: &ToolServer, byte: u8, expires: i64) -> SignInFlow {
    SignInFlow {
        id: Uuid::now_v7(),
        user_id: server.user_id(),
        server_id: server.id(),
        state_hash: vec![byte; 32],
        sealed: vec![byte, byte],
        created_at: at(0),
        expires_at: at(expires),
    }
}

#[sqlx::test(migrations = false)]
#[ignore = "requires Postgres: run make test-db"]
async fn a_flow_is_taken_once_and_replaced_by_the_next_of_its_server(pool: DbPool) {
    let store = store(pool).await;
    let owner = user(&store).await;
    let server = oauth_server(&store, owner).await;
    let first = flow(&server, 1, 10);
    SignInRepository::insert(&store, &first).await.unwrap();
    let second = flow(&server, 2, 10);
    SignInRepository::insert(&store, &second).await.unwrap();
    assert_eq!(
        SignInRepository::take(&store, &first.state_hash)
            .await
            .unwrap(),
        None,
        "replaced"
    );
    assert_eq!(
        SignInRepository::take(&store, &second.state_hash)
            .await
            .unwrap(),
        Some(second.clone())
    );
    assert_eq!(
        SignInRepository::take(&store, &second.state_hash)
            .await
            .unwrap(),
        None,
        "used up"
    );

    // Expired flows go when the next one starts, and a deleted server takes its flow along.
    let other = ToolServer::create(
        owner,
        "Web",
        Slug::parse("web").unwrap(),
        BaseUrl::parse("https://web.example.com", NetworkScope::Public).unwrap(),
        at(1),
    )
    .unwrap()
    .with_auth(AuthMode::OAuth);
    ToolServerRepository::insert(&store, &other).await.unwrap();
    let old = SignInFlow {
        created_at: at(0),
        expires_at: at(5),
        ..flow(&other, 3, 5)
    };
    SignInRepository::insert(&store, &old).await.unwrap();
    let later = SignInFlow {
        created_at: at(6),
        ..flow(&server, 4, 20)
    };
    SignInRepository::insert(&store, &later).await.unwrap();
    assert_eq!(
        SignInRepository::take(&store, &old.state_hash)
            .await
            .unwrap(),
        None,
        "purged"
    );
    ToolServerRepository::delete(&store, owner, server.id())
        .await
        .unwrap();
    assert_eq!(
        SignInRepository::take(&store, &later.state_hash)
            .await
            .unwrap(),
        None
    );
}

#[sqlx::test(migrations = false)]
#[ignore = "requires Postgres: run make test-db"]
async fn a_flow_needs_a_server_of_its_user(pool: DbPool) {
    let store = store(pool).await;
    let owner = user(&store).await;
    let other = user(&store).await;
    let server = oauth_server(&store, owner).await;
    let foreign = SignInFlow {
        user_id: other,
        ..flow(&server, 1, 10)
    };
    assert!(SignInRepository::insert(&store, &foreign).await.is_err());
}
