use super::*;

/// An account store plus a throw-away hbbs peer table holding one registered id.
async fn serve() -> (Arc<Accounts>, String, reqwest::Client) {
    let peer_db = std::env::temp_dir().join(format!("audit-peers-{}.sqlite3", uuid::Uuid::new_v4()));
    let peers = SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(SqliteConnectOptions::new().filename(&peer_db).create_if_missing(true))
        .await
        .unwrap();
    sqlx::query("CREATE TABLE peer (guid BLOB PRIMARY KEY NOT NULL, id VARCHAR(100) NOT NULL, uuid BLOB NOT NULL, pk BLOB NOT NULL, info TEXT NOT NULL) WITHOUT ROWID")
        .execute(&peers)
        .await
        .unwrap();
    sqlx::query("INSERT INTO peer(guid,id,uuid,pk,info) VALUES(x'01','100000002',x'00',x'00','{}')")
        .execute(&peers)
        .await
        .unwrap();
    drop(peers);
    let db = Arc::new(
        Accounts::open_with_peers(":memory:", Some(peer_db.to_str().unwrap()))
            .await
            .unwrap(),
    );
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(
        axum::Server::from_tcp(listener)
            .unwrap()
            .serve(router(db.clone()).into_make_service_with_connect_info::<SocketAddr>()),
    );
    let client = reqwest::Client::builder().no_proxy().build().unwrap();
    (db, format!("http://{}/api/audit/conn", addr), client)
}

async fn post(client: &reqwest::Client, url: &str, body: String) -> StatusCode {
    client.post(url).body(body).send().await.unwrap().status()
}

async fn count(db: &Accounts) -> i64 {
    sqlx::query("SELECT COUNT(*) FROM conn_audit")
        .fetch_one(&db.pool)
        .await
        .unwrap()
        .get::<i64, _>(0)
}

#[tokio::test]
async fn records_are_stored_once_per_nonce() {
    let (db, url, client) = serve().await;
    let record = json!({
        "id": "100000002", "uuid": "dGVzdA==", "conn_id": 839, "session_id": 17,
        "ip": "198.51.100.20", "action": "new", "nonce": "n-1", "conn_audit_ref": "ref-1"
    });
    assert_eq!(post(&client, &url, record.to_string()).await, StatusCode::OK);
    assert_eq!(post(&client, &url, record.to_string()).await, StatusCode::OK);
    assert_eq!(count(&db).await, 1, "retried post is deduplicated by nonce");
    let row = sqlx::query("SELECT id,peer_id,action,conn_id,session_id,note,from_ip FROM conn_audit WHERE nonce='n-1'")
        .fetch_one(&db.pool)
        .await
        .unwrap();
    assert_eq!(row.get::<String, _>(0), "100000002");
    assert_eq!(row.get::<Option<String>, _>(1), None);
    assert_eq!(row.get::<String, _>(2), "new");
    assert_eq!(row.get::<i64, _>(3), 839);
    assert_eq!(row.get::<i64, _>(4), 17);
    let note: Value = serde_json::from_str(&row.get::<String, _>(5)).unwrap();
    assert_eq!(note, json!({"uuid": "dGVzdA==", "conn_audit_ref": "ref-1"}));
    assert_eq!(row.get::<String, _>(6), "127.0.0.1", "server-side source address, not the body's ip");

    let logon = json!({
        "id": "100000002", "conn_id": 839, "session_id": 17, "nonce": "n-2",
        "peer": ["100000001", "desk"], "type": 0, "primary_auth": 1
    });
    assert_eq!(post(&client, &url, logon.to_string()).await, StatusCode::OK);
    let row = sqlx::query("SELECT peer_id,action FROM conn_audit WHERE nonce='n-2'")
        .fetch_one(&db.pool)
        .await
        .unwrap();
    assert_eq!(row.get::<String, _>(0), "100000001");
    assert_eq!(row.get::<Option<String>, _>(1), None);
}

#[tokio::test]
async fn unknown_peer_is_dropped() {
    let (db, url, client) = serve().await;
    let record = json!({"id": "999999999", "action": "new", "nonce": "n-3"});
    assert_eq!(post(&client, &url, record.to_string()).await, StatusCode::NO_CONTENT);
    assert_eq!(count(&db).await, 0);
}

#[tokio::test]
async fn bad_requests_are_refused() {
    let (db, url, client) = serve().await;
    assert_eq!(post(&client, &url, "{\"id\":\"100000002\"}".into()).await, StatusCode::BAD_REQUEST);
    assert_eq!(post(&client, &url, "{\"nonce\":\"n-4\"}".into()).await, StatusCode::BAD_REQUEST);
    assert_eq!(post(&client, &url, "not json".into()).await, StatusCode::BAD_REQUEST);
    let huge = json!({"id": "100000002", "nonce": "n-5", "note": "x".repeat(5000)}).to_string();
    assert_eq!(post(&client, &url, huge).await, StatusCode::PAYLOAD_TOO_LARGE);
    assert_eq!(count(&db).await, 0);
}

#[tokio::test]
async fn sixty_records_per_minute_per_address() {
    let (db, url, client) = serve().await;
    for i in 0..60 {
        let record = json!({"id": "100000002", "action": "new", "nonce": format!("r-{i}")});
        assert_eq!(post(&client, &url, record.to_string()).await, StatusCode::OK, "{i}");
    }
    let record = json!({"id": "100000002", "action": "new", "nonce": "r-60"});
    assert_eq!(post(&client, &url, record.to_string()).await, StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(count(&db).await, 60);
}
