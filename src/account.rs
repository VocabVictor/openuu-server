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

impl Accounts {
    pub async fn open(path: &str) -> ResultType<Self> {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with({
                let mut options = SqliteConnectOptions::new()
                    .filename(path)
                    .create_if_missing(true)
                    .busy_timeout(Duration::from_secs(5));
                options.log_statements(log::LevelFilter::Debug);
                options
            })
            .await?;
        sqlx::query("CREATE TABLE IF NOT EXISTS accounts (name TEXT PRIMARY KEY, password_hash TEXT NOT NULL, enabled INTEGER NOT NULL DEFAULT 1)").execute(&pool).await?;
        sqlx::query("CREATE TABLE IF NOT EXISTS account_sessions (token_hash BLOB PRIMARY KEY, name TEXT NOT NULL, expires INTEGER NOT NULL)").execute(&pool).await?;
        sqlx::query("CREATE INDEX IF NOT EXISTS session_expiry ON account_sessions(expires)")
            .execute(&pool)
            .await?;
        sqlx::query("CREATE TABLE IF NOT EXISTS relay_tickets (hash BLOB PRIMARY KEY, session_hash BLOB NOT NULL, relay_id TEXT NOT NULL, expires INTEGER NOT NULL)").execute(&pool).await?;
        wol::init(&pool).await?;
        Ok(Self {
            pool,
            attempts: Mutex::new(HashMap::new()),
            hashing: Arc::new(Semaphore::new(4)),
        })
    }
    fn valid_name(name: &str) -> bool {
        !name.is_empty()
            && name.len() <= 64
            && name
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || "._-".contains(c))
    }
    async fn hash_password(password: String) -> ResultType<String> {
        if password.len() < 12 || password.len() > 72 {
            hbb_common::bail!("Use a 12-72 byte password");
        }
        Ok(
            tokio::task::spawn_blocking(move || bcrypt::hash(password, bcrypt::DEFAULT_COST))
                .await??,
        )
    }
    pub async fn create_user(&self, name: &str, password: String) -> ResultType<()> {
        if !Self::valid_name(name) {
            hbb_common::bail!("Use a 1-64 character account name (letters, digits, . _ -)");
        }
        let hash = Self::hash_password(password).await?;
        sqlx::query("INSERT INTO accounts(name,password_hash) VALUES(?,?)")
            .bind(name)
            .bind(hash)
            .execute(&self.pool)
            .await?;
        log::info!("event=account_created user={name}");
        Ok(())
    }
    pub async fn set_password(&self, name: &str, password: String) -> ResultType<()> {
        let hash = Self::hash_password(password).await?;
        let result = sqlx::query("UPDATE accounts SET password_hash=? WHERE name=?")
            .bind(hash)
            .bind(name)
            .execute(&self.pool)
            .await?;
        if result.rows_affected() == 0 {
            hbb_common::bail!("No such account: {name}");
        }
        self.revoke_user(name).await?;
        log::info!("event=password_changed user={name}");
        Ok(())
    }
    pub async fn set_enabled(&self, name: &str, enabled: bool) -> ResultType<()> {
        let result = sqlx::query("UPDATE accounts SET enabled=? WHERE name=?")
            .bind(enabled as i64)
            .bind(name)
            .execute(&self.pool)
            .await?;
        if result.rows_affected() == 0 {
            hbb_common::bail!("No such account: {name}");
        }
        if !enabled {
            self.revoke_user(name).await?;
        }
        log::info!(
            "event=account_{} user={name}",
            if enabled { "enabled" } else { "disabled" }
        );
        Ok(())
    }
    pub async fn delete_user(&self, name: &str) -> ResultType<()> {
        self.revoke_user(name).await?;
        sqlx::query("DELETE FROM wol_jobs WHERE owner=?")
            .bind(name)
            .execute(&self.pool)
            .await?;
        sqlx::query("DELETE FROM wol_devices WHERE owner=?")
            .bind(name)
            .execute(&self.pool)
            .await?;
        let result = sqlx::query("DELETE FROM accounts WHERE name=?")
            .bind(name)
            .execute(&self.pool)
            .await?;
        if result.rows_affected() == 0 {
            hbb_common::bail!("No such account: {name}");
        }
        log::info!("event=account_deleted user={name}");
        Ok(())
    }
    pub async fn list_users(&self) -> ResultType<Vec<AccountSummary>> {
        let rows = sqlx::query("SELECT a.name AS name, a.enabled AS enabled, (SELECT COUNT(*) FROM account_sessions s WHERE s.name=a.name AND s.expires>?) AS sessions FROM accounts a ORDER BY a.name")
            .bind(now())
            .fetch_all(&self.pool)
            .await?;
        Ok(rows
            .iter()
            .map(|r| AccountSummary {
                name: r.get("name"),
                enabled: r.get::<i64, _>("enabled") != 0,
                sessions: r.get::<i64, _>("sessions"),
            })
            .collect())
    }
    async fn revoke_user(&self, name: &str) -> ResultType<()> {
        sqlx::query("DELETE FROM relay_tickets WHERE session_hash IN (SELECT token_hash FROM account_sessions WHERE name=?)")
            .bind(name)
            .execute(&self.pool)
            .await?;
        sqlx::query("DELETE FROM account_sessions WHERE name=?")
            .bind(name)
            .execute(&self.pool)
            .await?;
        Ok(())
    }
    pub async fn user(&self, token: &str) -> ResultType<Option<String>> {
        if token.len() != 64 {
            return Ok(None);
        }
        Ok(sqlx::query("SELECT a.name FROM account_sessions s JOIN accounts a ON a.name=s.name WHERE s.token_hash=? AND s.expires>? AND a.enabled=1")
            .bind(digest(token)).bind(now()).fetch_optional(&self.pool).await?.map(|r| r.get("name")))
    }
    async fn login(&self, name: String, password: String) -> ResultType<Option<String>> {
        if name.len() > 64 || password.len() > 72 {
            return Ok(None);
        }
        let row = sqlx::query("SELECT password_hash FROM accounts WHERE name=? AND enabled=1")
            .bind(&name)
            .fetch_optional(&self.pool)
            .await?;
        let hash: Option<String> = row.map(|r| r.get("password_hash"));
        let permit = self.hashing.clone().acquire_owned().await?;
        let valid = tokio::task::spawn_blocking(move || {
            let _permit = permit;
            match hash {
                Some(hash) => bcrypt::verify(password, &hash).unwrap_or(false),
                None => {
                    let _ = bcrypt::hash(password, bcrypt::DEFAULT_COST);
                    false
                }
            }
        })
        .await?;
        if !valid {
            return Ok(None);
        }
        let token = format!(
            "{}{}",
            uuid::Uuid::new_v4().simple(),
            uuid::Uuid::new_v4().simple()
        );
        sqlx::query("DELETE FROM account_sessions WHERE expires<=?")
            .bind(now())
            .execute(&self.pool)
            .await?;
        sqlx::query("INSERT INTO account_sessions(token_hash,name,expires) VALUES(?,?,?)")
            .bind(digest(&token))
            .bind(name)
            .bind(now() + 86400)
            .execute(&self.pool)
            .await?;
        Ok(Some(token))
    }
    pub async fn ticket(&self, token: &str, relay_id: &str) -> ResultType<Option<String>> {
        if relay_id.is_empty() || relay_id.len() > 128 || self.user(token).await?.is_none() {
            return Ok(None);
        }
        let ticket = format!(
            "{}{}",
            uuid::Uuid::new_v4().simple(),
            uuid::Uuid::new_v4().simple()
        );
        sqlx::query("DELETE FROM relay_tickets WHERE expires<=?")
            .bind(now())
            .execute(&self.pool)
            .await?;
        sqlx::query(
            "INSERT INTO relay_tickets(hash,session_hash,relay_id,expires) VALUES(?,?,?,?)",
        )
        .bind(digest(&ticket))
        .bind(digest(token))
        .bind(relay_id)
        .bind(now() + 60)
        .execute(&self.pool)
        .await?;
        Ok(Some(ticket))
    }
    pub async fn redeem(&self, ticket: &str, relay_id: &str) -> ResultType<bool> {
        if ticket.len() != 64 {
            return Ok(false);
        }
        let result=sqlx::query("DELETE FROM relay_tickets WHERE hash=? AND relay_id=? AND expires>? AND session_hash IN (SELECT s.token_hash FROM account_sessions s JOIN accounts a ON a.name=s.name WHERE s.expires>? AND a.enabled=1)")
            .bind(digest(ticket)).bind(relay_id).bind(now()).bind(now()).execute(&self.pool).await?;
        Ok(result.rows_affected() == 1)
    }
    pub async fn revoke(&self, token: &str) -> ResultType<()> {
        sqlx::query("DELETE FROM account_sessions WHERE token_hash=?")
            .bind(digest(token))
            .execute(&self.pool)
            .await?;
        Ok(())
    }
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
async fn login(
    Extension(db): Extension<Arc<Accounts>>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    body: axum::body::Bytes,
) -> ApiResult {
    if body.len() > 16384 {
        return Err(error(StatusCode::PAYLOAD_TOO_LARGE, "Request too large"));
    }
    {
        let mut attempts = db.attempts.lock().await;
        attempts.retain(|_, (time, _)| now() - *time < 60);
        if attempts.len() >= 10000 && !attempts.contains_key(&addr.ip()) {
            return Err(error(StatusCode::TOO_MANY_REQUESTS, "Try again later"));
        }
        let entry = attempts.entry(addr.ip()).or_insert((now(), 0));
        entry.1 += 1;
        if entry.1 > 10 {
            log::warn!("event=login_rate_limited ip={}", addr.ip());
            return Err(error(StatusCode::TOO_MANY_REQUESTS, "Try again later"));
        }
    }
    let input: Credentials = serde_json::from_slice(&body)
        .map_err(|_| error(StatusCode::BAD_REQUEST, "Invalid request"))?;
    let token = db
        .login(input.username.clone(), input.password)
        .await
        .map_err(|err| {
            log::error!("event=login_error ip={} err={err}", addr.ip());
            error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Authentication unavailable",
            )
        })?
        .ok_or_else(|| {
            log::warn!(
                "event=login_failed ip={} user={:?}",
                addr.ip(),
                input.username.chars().take(64).collect::<String>()
            );
            error(StatusCode::UNAUTHORIZED, "Invalid username or password")
        })?;
    log::info!("event=login_ok ip={} user={}", addr.ip(), input.username);
    Ok(Json(
        json!({"type":"access_token","access_token":token,"user":profile(&input.username)}),
    ))
}
async fn current(Extension(db): Extension<Arc<Accounts>>, headers: HeaderMap) -> ApiResult {
    let name = db
        .user(bearer(&headers))
        .await
        .map_err(|_| {
            error(
                StatusCode::SERVICE_UNAVAILABLE,
                "Authentication unavailable",
            )
        })?
        .ok_or_else(|| error(StatusCode::UNAUTHORIZED, "Login required"))?;
    Ok(Json(profile(&name)))
}
async fn logout(Extension(db): Extension<Arc<Accounts>>, headers: HeaderMap) -> ApiResult {
    db.revoke(bearer(&headers)).await.map_err(|_| {
        error(
            StatusCode::SERVICE_UNAVAILABLE,
            "Authentication unavailable",
        )
    })?;
    Ok(Json(json!({})))
}
async fn empty_directory(Extension(db): Extension<Arc<Accounts>>, headers: HeaderMap) -> ApiResult {
    if db
        .user(bearer(&headers))
        .await
        .map_err(|_| {
            error(
                StatusCode::SERVICE_UNAVAILABLE,
                "Authentication unavailable",
            )
        })?
        .is_none()
    {
        return Err(error(StatusCode::UNAUTHORIZED, "Login required"));
    }
    Ok(Json(json!({"total":0,"data":[]})))
}
async fn address_book(Extension(db): Extension<Arc<Accounts>>, headers: HeaderMap) -> ApiResult {
    if db
        .user(bearer(&headers))
        .await
        .map_err(|_| {
            error(
                StatusCode::SERVICE_UNAVAILABLE,
                "Authentication unavailable",
            )
        })?
        .is_none()
    {
        return Err(error(StatusCode::UNAUTHORIZED, "Login required"));
    }
    Ok(Json(Value::Null))
}
async fn relay_ticket(
    Extension(db): Extension<Arc<Accounts>>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> ApiResult {
    let ticket = db
        .ticket(
            bearer(&headers),
            body.get("uuid").and_then(Value::as_str).unwrap_or(""),
        )
        .await
        .map_err(|_| {
            error(
                StatusCode::SERVICE_UNAVAILABLE,
                "Authentication unavailable",
            )
        })?
        .ok_or_else(|| {
            log::warn!("event=relay_ticket_denied");
            error(StatusCode::UNAUTHORIZED, "Login required")
        })?;
    log::info!(
        "event=relay_ticket_issued relay={}",
        body.get("uuid").and_then(Value::as_str).unwrap_or("")
    );
    Ok(Json(json!({"ticket":ticket})))
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
    Router::new()
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
        .layer(Extension(db))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn session_lifecycle() {
        let db = Accounts::open(":memory:").await.unwrap();
        db.create_user("tester", "long-test-password".into())
            .await
            .unwrap();
        assert!(db
            .login("tester".into(), "incorrect".into())
            .await
            .unwrap()
            .is_none());
        let token = db
            .login("tester".into(), "long-test-password".into())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(db.user(&token).await.unwrap().as_deref(), Some("tester"));
        assert!(db.user("").await.unwrap().is_none());
        assert!(db.user(&"a".repeat(64)).await.unwrap().is_none());
        let ticket = db.ticket(&token, "relay-1").await.unwrap().unwrap();
        assert!(!db.redeem(&ticket, "relay-2").await.unwrap());
        assert!(db.redeem(&ticket, "relay-1").await.unwrap());
        assert!(!db.redeem(&ticket, "relay-1").await.unwrap());
        let ticket = db.ticket(&token, "relay-1").await.unwrap().unwrap();
        db.revoke(&token).await.unwrap();
        assert!(!db.redeem(&ticket, "relay-1").await.unwrap());
        assert!(db.user(&token).await.unwrap().is_none());
        let token = db
            .login("tester".into(), "long-test-password".into())
            .await
            .unwrap()
            .unwrap();
        sqlx::query("UPDATE account_sessions SET expires=0")
            .execute(&db.pool)
            .await
            .unwrap();
        assert!(db.user(&token).await.unwrap().is_none());
    }
    #[tokio::test]
    async fn admin_commands() {
        let db = Accounts::open(":memory:").await.unwrap();
        assert!(db.create_user("bad name", "long-test-password".into()).await.is_err());
        assert!(db.create_user("short", "short".into()).await.is_err());
        db.create_user("alice", "long-test-password".into()).await.unwrap();
        db.create_user("bob", "long-test-password".into()).await.unwrap();
        assert!(db.create_user("alice", "long-test-password".into()).await.is_err());
        let token = db.login("alice".into(), "long-test-password".into()).await.unwrap().unwrap();
        let ticket = db.ticket(&token, "relay-1").await.unwrap().unwrap();
        let listed = db.list_users().await.unwrap();
        assert_eq!(listed.len(), 2);
        assert_eq!((listed[0].name.as_str(), listed[0].enabled, listed[0].sessions), ("alice", true, 1));
        assert_eq!((listed[1].name.as_str(), listed[1].sessions), ("bob", 0));

        assert!(db.set_password("nobody", "long-test-password".into()).await.is_err());
        assert!(db.set_password("alice", "short".into()).await.is_err());
        db.set_password("alice", "another-long-password".into()).await.unwrap();
        assert!(db.user(&token).await.unwrap().is_none());
        assert!(!db.redeem(&ticket, "relay-1").await.unwrap());
        assert!(db.login("alice".into(), "long-test-password".into()).await.unwrap().is_none());
        let token = db.login("alice".into(), "another-long-password".into()).await.unwrap().unwrap();

        assert!(db.set_enabled("nobody", false).await.is_err());
        db.set_enabled("alice", false).await.unwrap();
        assert!(db.user(&token).await.unwrap().is_none());
        assert!(db.login("alice".into(), "another-long-password".into()).await.unwrap().is_none());
        assert!(!db.list_users().await.unwrap()[0].enabled);
        db.set_enabled("alice", true).await.unwrap();
        assert!(db.login("alice".into(), "another-long-password".into()).await.unwrap().is_some());

        sqlx::query("INSERT INTO wol_devices(owner,id,seen,session,peers) VALUES('alice','dev',0,x'00','[]')")
            .execute(&db.pool).await.unwrap();
        assert!(db.delete_user("nobody").await.is_err());
        db.delete_user("alice").await.unwrap();
        let listed = db.list_users().await.unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].name, "bob");
        let leftover: i64 = sqlx::query("SELECT COUNT(*) AS n FROM wol_devices WHERE owner='alice'")
            .fetch_one(&db.pool).await.unwrap().get("n");
        assert_eq!(leftover, 0);
        assert!(db.login("alice".into(), "another-long-password".into()).await.unwrap().is_none());
    }
}

#[cfg(test)]
mod http_tests {
    use super::*;
    #[tokio::test]
    async fn client_api_and_revocation() {
        let db = Arc::new(Accounts::open(":memory:").await.unwrap());
        db.create_user("apitest", "test-only-long-password".into())
            .await
            .unwrap();
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let server = tokio::spawn(
            axum::Server::from_tcp(listener)
                .unwrap()
                .serve(router(db).into_make_service_with_connect_info::<SocketAddr>()),
        );
        let client = reqwest::Client::new();
        let base = format!("http://{}", addr);
        assert_eq!(
            client
                .post(format!("{base}/api/currentUser"))
                .send()
                .await
                .unwrap()
                .status(),
            StatusCode::UNAUTHORIZED
        );
        assert_eq!(
            client
                .get(format!("{base}/api/login-options"))
                .send()
                .await
                .unwrap()
                .json::<Value>()
                .await
                .unwrap(),
            json!([])
        );
        let response = client
            .post(format!("{base}/api/login"))
            .body(json!({"username":"apitest","password":"test-only-long-password"}).to_string())
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let login: Value = response.json().await.unwrap();
        let token = login["access_token"].as_str().unwrap();
        assert_eq!(login["type"], "access_token");
        assert_eq!(
            client
                .post(format!("{base}/api/currentUser"))
                .bearer_auth(token)
                .send()
                .await
                .unwrap()
                .status(),
            StatusCode::OK
        );
        assert_eq!(
            client
                .post(format!("{base}/api/relay-ticket"))
                .bearer_auth(token)
                .json(&json!({"uuid":"test-relay"}))
                .send()
                .await
                .unwrap()
                .status(),
            StatusCode::OK
        );
        assert_eq!(
            client
                .post(format!("{base}/api/logout"))
                .bearer_auth(token)
                .send()
                .await
                .unwrap()
                .status(),
            StatusCode::OK
        );
        assert_eq!(
            client
                .post(format!("{base}/api/currentUser"))
                .bearer_auth(token)
                .send()
                .await
                .unwrap()
                .status(),
            StatusCode::UNAUTHORIZED
        );
        for _ in 0..10 {
            let _ = client
                .post(format!("{base}/api/login"))
                .body("invalid")
                .send()
                .await
                .unwrap();
        }
        assert_eq!(
            client
                .post(format!("{base}/api/login"))
                .body("invalid")
                .send()
                .await
                .unwrap()
                .status(),
            StatusCode::TOO_MANY_REQUESTS
        );
        server.abort();
    }
}
