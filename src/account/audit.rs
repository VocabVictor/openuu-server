use super::*;
use std::net::IpAddr;

/// Connection audit records posted by the controlled peer
/// (`src/server/connection/audit.rs` in the client) to `/api/audit/conn`.
///
/// The poster is an unattended machine without a session, so there is no
/// bearer token to check. Trust comes from the server side instead: the
/// reporting `id` must be registered in the hbbs peer table (`OPENUU_PEER_DB`),
/// `from_ip` and `received_at` are filled by the server, the body is capped at
/// 4 KB and 60 records per minute per source address, and `nonce` is unique so
/// the client's retries store nothing twice. Client fields are stored by a
/// whitelist; everything else is kept verbatim in `note`.
pub(super) async fn init(pool: &SqlitePool) -> ResultType<()> {
    sqlx::query("CREATE TABLE IF NOT EXISTS conn_audit(nonce TEXT PRIMARY KEY, id TEXT NOT NULL, peer_id TEXT, action TEXT, conn_id INTEGER, session_id INTEGER, note TEXT, from_ip TEXT NOT NULL, received_at INTEGER NOT NULL)").execute(pool).await?;
    Ok(())
}

const MAX_BODY: usize = 4096;
const PER_MINUTE: u32 = 60;

fn text(v: &Value, key: &str) -> Option<String> {
    match v.get(key)? {
        Value::String(s) => Some(s.chars().take(256).collect()),
        Value::Null => None,
        other => Some(other.to_string().chars().take(256).collect()),
    }
}

async fn over_limit(db: &Accounts, ip: IpAddr) -> bool {
    let mut rate = db.audit_rate.lock().await;
    rate.retain(|_, (since, _)| now() - *since < 60);
    if rate.len() >= 10000 && !rate.contains_key(&ip) {
        return true;
    }
    let entry = rate.entry(ip).or_insert((now(), 0));
    entry.1 += 1;
    entry.1 > PER_MINUTE
}

async fn registered(db: &Accounts, id: &str) -> bool {
    let Some(peers) = db.peers.as_ref() else {
        return false;
    };
    match sqlx::query("SELECT 1 FROM peer WHERE id=?")
        .bind(id)
        .fetch_optional(peers)
        .await
    {
        Ok(row) => row.is_some(),
        Err(err) => {
            log::error!("event=audit_peer_lookup_error err={err}");
            false
        }
    }
}

pub(super) async fn conn(
    Extension(db): Extension<Arc<Accounts>>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    body: axum::body::Bytes,
) -> Result<StatusCode, (StatusCode, Json<Value>)> {
    if over_limit(&db, addr.ip()).await {
        return Err(error(StatusCode::TOO_MANY_REQUESTS, "Try again later"));
    }
    if body.len() > MAX_BODY {
        return Err(error(StatusCode::PAYLOAD_TOO_LARGE, "Audit record too large"));
    }
    let v: Value = serde_json::from_slice(&body)
        .map_err(|_| error(StatusCode::BAD_REQUEST, "Invalid audit record"))?;
    let (Some(nonce), Some(id)) = (
        text(&v, "nonce").filter(|n| !n.is_empty()),
        text(&v, "id").filter(|n| !n.is_empty()),
    ) else {
        return Err(error(StatusCode::BAD_REQUEST, "Missing nonce or id"));
    };
    if !registered(&db, &id).await {
        log::warn!("event=conn_audit_dropped id={id} from={} reason=unknown_peer", addr.ip());
        return Ok(StatusCode::NO_CONTENT);
    }
    // "peer" arrives as [id, name] on the logon record.
    let peer_id = v
        .get("peer")
        .and_then(|p| p.get(0))
        .and_then(Value::as_str)
        .map(|s| s.chars().take(256).collect::<String>());
    let mut note = v.clone();
    if let Some(obj) = note.as_object_mut() {
        for key in ["nonce", "id", "ip", "action", "conn_id", "session_id"] {
            obj.remove(key);
        }
    }
    let inserted = sqlx::query("INSERT OR IGNORE INTO conn_audit(nonce,id,peer_id,action,conn_id,session_id,note,from_ip,received_at) VALUES(?,?,?,?,?,?,?,?,?)")
        .bind(&nonce)
        .bind(&id)
        .bind(peer_id)
        .bind(text(&v, "action"))
        .bind(v.get("conn_id").and_then(Value::as_i64))
        .bind(v.get("session_id").and_then(Value::as_i64))
        .bind(note.to_string())
        .bind(addr.ip().to_string())
        .bind(now())
        .execute(&db.pool)
        .await
        .map_err(|err| {
            log::error!("event=audit_store_error err={err}");
            error(StatusCode::SERVICE_UNAVAILABLE, "Audit store unavailable")
        })?
        .rows_affected();
    if inserted == 1 {
        log::info!(
            "event=conn_audit id={id} action={} conn={} from={}",
            text(&v, "action").unwrap_or_default(),
            v.get("conn_id").and_then(Value::as_i64).unwrap_or_default(),
            addr.ip()
        );
    }
    Ok(StatusCode::OK)
}
