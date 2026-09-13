use super::ticket::{check_url, Redeemer};
use hbb_common::tokio;
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};

const SECRET: &str = "test-only-internal-secret";

#[test]
fn url_must_be_https_or_private_http() {
    for ok in [
        "https://account.example.com",
        "https://203.0.113.5:21114",
        "http://127.0.0.1:21114",
        "http://10.0.0.5:21114",
        "http://172.16.3.4",
        "http://192.168.100.5:21114/",
        "http://[::1]:21114",
        "http://[fd00::5]:21114",
    ] {
        assert!(check_url(ok).is_ok(), "{ok}");
    }
    for bad in [
        "http://account.example.com",
        "http://localhost:21114",
        "http://203.0.113.5:21114",
        "http://[2001:db8::1]:21114",
        "ftp://10.0.0.5",
        "10.0.0.5:21114",
    ] {
        assert!(check_url(bad).is_err(), "{bad}");
    }
    assert!(Redeemer::http("http://10.0.0.5:21114", "short").is_err());
}

/// A stand-in account service that answers every redeem with `reply` after `delay_ms`.
async fn account(reply: &'static str, delay_ms: u64) -> (String, Arc<AtomicUsize>) {
    use axum::{routing::post, Router};
    let hits = Arc::new(AtomicUsize::new(0));
    let counter = hits.clone();
    let app = Router::new().route(
        "/api/internal/relay-ticket/redeem",
        post(move |headers: axum::http::HeaderMap, body: String| {
            let counter = counter.clone();
            async move {
                counter.fetch_add(1, Ordering::SeqCst);
                assert_eq!(headers.get("x-openuu-internal").unwrap(), SECRET);
                assert!(body.contains("\"ticket\""));
                tokio::time::sleep(std::time::Duration::from_millis(delay_ms)).await;
                ([("content-type", "application/json")], reply)
            }
        }),
    );
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(axum::Server::from_tcp(listener).unwrap().serve(app.into_make_service()));
    (format!("http://{addr}"), hits)
}

#[tokio::test]
async fn ok_true_accepts() {
    let (url, hits) = account(r#"{"ok":true}"#, 0).await;
    let r = Redeemer::http(&url, SECRET).unwrap();
    assert!(r.redeem(&"a".repeat(64), "relay-1").await);
    assert_eq!(hits.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn ok_false_denies_without_retry() {
    let (url, hits) = account(r#"{"ok":false,"reason":"unknown"}"#, 0).await;
    let r = Redeemer::http(&url, SECRET).unwrap();
    assert!(!r.redeem(&"a".repeat(64), "relay-1").await);
    assert_eq!(hits.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn malformed_body_denies() {
    let (url, _) = account("not json", 0).await;
    let r = Redeemer::http(&url, SECRET).unwrap();
    assert!(!r.redeem(&"a".repeat(64), "relay-1").await);
}

#[tokio::test]
async fn slow_server_denies_without_retry() {
    let (url, hits) = account(r#"{"ok":true}"#, 4_000).await;
    let r = Redeemer::http(&url, SECRET).unwrap();
    let started = std::time::Instant::now();
    assert!(!r.redeem(&"a".repeat(64), "relay-1").await);
    assert!(started.elapsed() < std::time::Duration::from_secs(4));
    assert_eq!(hits.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn refused_connection_denies() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    drop(listener);
    let r = Redeemer::http(&url, SECRET).unwrap();
    assert!(!r.redeem(&"a".repeat(64), "relay-1").await);
}
