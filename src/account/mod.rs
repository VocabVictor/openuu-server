mod wol;
use axum::{
    extract::{ConnectInfo, Extension},
    http::{HeaderMap, StatusCode},
    routing::{get, post},
    Json, Router,
};
use hbb_common::{log, tokio, ResultType};
use serde::Deserialize;
use serde_json::{json, Value};
use sqlx::{
    sqlite::{SqliteConnectOptions, SqlitePoolOptions},
    ConnectOptions, Row, SqlitePool,
};
use std::{
    collections::HashMap,
    net::SocketAddr,
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tokio::sync::{Mutex, Semaphore};

mod store;
mod api;
pub mod internal;
use api::{address_book, current, empty_directory, login, logout, relay_ticket};
#[cfg(test)]
mod tests;
#[cfg(test)]
mod http_tests;
#[cfg(test)]
mod internal_tests;

pub struct Accounts {
    pool: SqlitePool,
    attempts: Mutex<HashMap<std::net::IpAddr, (i64, u32)>>,
    hashing: Arc<Semaphore>,
}
pub struct AccountSummary {
    pub name: String,
    pub enabled: bool,
    pub sessions: i64,
}
type ApiResult = Result<Json<Value>, (StatusCode, Json<Value>)>;
fn error(code: StatusCode, message: &str) -> (StatusCode, Json<Value>) {
    (code, Json(json!({"error": message})))
}
fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|v| v.as_secs() as i64)
        .unwrap_or_default()
}
fn digest(token: &str) -> Vec<u8> {
    sodiumoxide::crypto::hash::sha256::hash(token.as_bytes())
        .0
        .to_vec()
}

pub async fn shared() -> ResultType<&'static Accounts> {
    static STORE: tokio::sync::OnceCell<Accounts> = tokio::sync::OnceCell::const_new();
    STORE
        .get_or_try_init(|| async {
            Accounts::open(
                &std::env::var("OPENUU_ACCOUNT_DB")
                    .unwrap_or_else(|_| "openuu-accounts.sqlite3".into()),
            )
            .await
        })
        .await
}
pub async fn authorized(token: &str) -> bool {
    match shared().await {
        Ok(db) => match db.user(token).await {
            Ok(Some(_)) => true,
            Ok(None) => false,
            Err(err) => {
                log::error!("event=session_check_error err={err}");
                false
            }
        },
        Err(err) => {
            log::error!("event=account_db_unavailable err={err}");
            false
        }
    }
}
fn bearer(headers: &HeaderMap) -> &str {
    headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .unwrap_or("")
}
fn profile(name: &str) -> Value {
    json!({"name":name,"display_name":name,"status":1,"is_admin":false})
}
#[derive(Deserialize)]
struct Credentials {
    username: String,
    password: String,
}
pub async fn redeem_ticket(ticket: &str, relay_id: &str) -> bool {
    match shared().await {
        Ok(db) => match db.redeem(ticket, relay_id).await {
            Ok(true) => true,
            Ok(false) => {
                log::warn!("event=relay_ticket_rejected relay={relay_id}");
                false
            }
            Err(err) => {
                log::error!("event=relay_ticket_error relay={relay_id} err={err}");
                false
            }
        },
        Err(err) => {
            log::error!("event=account_db_unavailable err={err}");
            false
        }
    }
}
pub fn router(db: Arc<Accounts>) -> Router {
    router_with_internal(db, internal::Secret::from_env())
}
pub fn router_with_internal(db: Arc<Accounts>, secret: Option<internal::Secret>) -> Router {
    let router = Router::new()
        .route("/api/wol/poll", post(wol::poll))
        .route("/api/wol/status", post(wol::status))
        .route("/api/wol/wake", post(wol::wake))
        .route("/api/ab", get(address_book))
        .route("/api/users", get(empty_directory))
        .route("/api/peers", get(empty_directory))
        .route("/api/device-group/accessible", get(empty_directory))
        .route("/api/relay-ticket", post(relay_ticket))
        .route("/api/login", post(login))
        .route("/api/currentUser", post(current).get(current))
        .route("/api/logout", post(logout))
        .route("/api/login-options", get(|| async { Json(json!([])) }))
        .layer(Extension(db.clone()));
    internal::attach(router, db, secret)
}
