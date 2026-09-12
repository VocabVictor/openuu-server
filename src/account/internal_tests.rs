use super::*;
use internal::{Secret, HEADER, PATH};

const SECRET: &str = "test-only-internal-secret";

async fn serve(secret: Option<Secret>) -> (Arc<Accounts>, String, reqwest::Client) {
    let db = Arc::new(Accounts::open(":memory:").await.unwrap());
    db.create_user("relaytest", "test-only-long-password".into())
        .await
        .unwrap();
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(
        axum::Server::from_tcp(listener).unwrap().serve(
            router_with_internal(db.clone(), secret)
                .into_make_service_with_connect_info::<SocketAddr>(),
        ),
    );
    let client = reqwest::Client::builder().no_proxy().build().unwrap();
    (db, format!("http://{}{}", addr, PATH), client)
}

async fn issue(db: &Accounts, relay_id: &str) -> String {
    let token = db
        .login("relaytest".into(), "test-only-long-password".into())
        .await
        .unwrap()
        .unwrap();
    db.ticket(&token, relay_id).await.unwrap().unwrap()
}

async fn redeem(client: &reqwest::Client, url: &str, secret: &str, body: Value) -> (StatusCode, Value) {
    let res = client
        .post(url)
        .header(HEADER, secret)
        .body(body.to_string())
        .send()
        .await
        .unwrap();
    let status = res.status();
    (status, res.json().await.unwrap_or(Value::Null))
}

#[tokio::test]
async fn ticket_redeems_once_with_the_right_relay() {
    let (db, url, client) = serve(Secret::new(SECRET)).await;
    let ticket = issue(&db, "relay-1").await;
    let body = json!({"ticket": ticket, "relay_id": "relay-1"});
    assert_eq!(
        redeem(&client, &url, SECRET, body.clone()).await,
        (StatusCode::OK, json!({"ok": true}))
    );
    assert_eq!(
        redeem(&client, &url, SECRET, body).await,
        (StatusCode::OK, json!({"ok": false, "reason": "unknown"}))
    );
}

#[tokio::test]
async fn refusals_name_their_reason() {
    let (db, url, client) = serve(Secret::new(SECRET)).await;
    let ticket = issue(&db, "relay-1").await;
    assert_eq!(
        redeem(&client, &url, SECRET, json!({"ticket": ticket, "relay_id": "relay-2"})).await.1,
        json!({"ok": false, "reason": "relay_mismatch"})
    );
    assert_eq!(
        redeem(&client, &url, SECRET, json!({"ticket": "abc", "relay_id": "relay-1"})).await.1,
        json!({"ok": false, "reason": "malformed"})
    );
    assert_eq!(
        redeem(&client, &url, SECRET, json!({"nope": 1})).await.1,
        json!({"ok": false, "reason": "malformed"})
    );
    sqlx::query("UPDATE relay_tickets SET expires=1")
        .execute(&db.pool)
        .await
        .unwrap();
    assert_eq!(
        redeem(&client, &url, SECRET, json!({"ticket": ticket, "relay_id": "relay-1"})).await.1,
        json!({"ok": false, "reason": "expired"})
    );
}

#[tokio::test]
async fn wrong_secret_is_unauthorized_and_consumes_nothing() {
    let (db, url, client) = serve(Secret::new(SECRET)).await;
    let ticket = issue(&db, "relay-1").await;
    let body = json!({"ticket": ticket, "relay_id": "relay-1"});
    assert_eq!(redeem(&client, &url, "wrong", body.clone()).await.0, StatusCode::UNAUTHORIZED);
    assert_eq!(
        client.post(&url).body(body.to_string()).send().await.unwrap().status(),
        StatusCode::UNAUTHORIZED
    );
    assert!(db.redeem(&ticket, "relay-1").await.unwrap());
}

#[tokio::test]
async fn route_is_absent_without_a_secret() {
    let (_, url, client) = serve(None).await;
    assert_eq!(
        client.post(&url).header(HEADER, SECRET).body("{}").send().await.unwrap().status(),
        StatusCode::NOT_FOUND
    );
    assert!(Secret::new("short").is_none());
}
