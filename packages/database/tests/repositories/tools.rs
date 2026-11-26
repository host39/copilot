use std::collections::BTreeMap;
use std::time::Duration;

use copilot_core::{
    ApprovalMode, BaseUrl, NetworkScope, RemoteTool, Slug, ToolListing, ToolMode, ToolServer,
    ToolServerRepository,
};
use copilot_database::DbPool;
use serde_json::json;

use crate::{at, store, user};

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
    store.insert(&server).await.unwrap();
    let bare = ToolServer::create(
        owner,
        "Web",
        Slug::parse("web").unwrap(),
        BaseUrl::parse("https://web.example.com", NetworkScope::Public).unwrap(),
        at(1),
    )
    .unwrap();
    store.insert(&bare).await.unwrap();

    assert_eq!(
        store.list(owner).await.unwrap(),
        vec![server.clone(), bare],
        "oldest first"
    );
    assert_eq!(
        store.get(owner, server.id()).await.unwrap(),
        Some(server.clone())
    );
    assert_eq!(store.get(other, server.id()).await.unwrap(), None);
    assert!(store.list(other).await.unwrap().is_empty());
}
