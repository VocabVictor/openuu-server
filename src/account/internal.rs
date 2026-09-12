use super::*;

/// Internal relay-ticket redemption used by an `hbbr` that runs on another host
/// (docs/relay-ticket-http.md). Registered only when `OPENUU_INTERNAL_SECRET`
/// (or `OPENUU_INTERNAL_SECRET_FILE`) is set, so deployments without a remote
/// relay expose nothing new.
pub const HEADER: &str = "x-openuu-internal";
pub const PATH: &str = "/api/internal/relay-ticket/redeem";

#[derive(Clone)]
pub struct Secret(Arc<Vec<u8>>);

impl Secret {
    pub fn from_env() -> Option<Self> {
        let value = match std::env::var("OPENUU_INTERNAL_SECRET_FILE") {
            Ok(path) => match std::fs::read_to_string(&path) {
                Ok(v) => v,
                Err(err) => {
                    log::error!("event=internal_secret_unreadable path={path} err={err}");
                    return None;
                }
            },
            Err(_) => std::env::var("OPENUU_INTERNAL_SECRET").ok()?,
        };
        Self::new(value.trim())
    }
    pub fn new(value: &str) -> Option<Self> {
        if value.len() < 16 {
            log::error!("event=internal_secret_too_short min=16");
            return None;
        }
        Some(Self(Arc::new(value.as_bytes().to_vec())))
    }
    fn matches(&self, headers: &HeaderMap) -> bool {
        headers
            .get(HEADER)
            .map(|v| v.as_bytes())
            .map_or(false, |v| sodiumoxide::utils::memcmp(v, &self.0))
    }
}

pub fn attach(router: Router, db: Arc<Accounts>, secret: Option<Secret>) -> Router {
    match secret {
        Some(secret) => {
            log::info!("event=internal_relay_ticket_enabled path={PATH}");
            router
                .route(PATH, post(redeem))
                .layer(Extension(db))
                .layer(Extension(secret))
        }
        None => router,
    }
}

#[derive(Deserialize)]
struct Redeem {
    ticket: String,
    relay_id: String,
}

async fn redeem(
    Extension(db): Extension<Arc<Accounts>>,
    Extension(secret): Extension<Secret>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> ApiResult {
    if !secret.matches(&headers) {
        log::warn!("event=internal_relay_ticket_unauthorized ip={}", addr.ip());
        return Err(error(StatusCode::UNAUTHORIZED, "Bad internal secret"));
    }
    let Ok(input) = serde_json::from_slice::<Redeem>(&body) else {
        return Ok(Json(json!({"ok": false, "reason": "malformed"})));
    };
    if input.ticket.len() != 64 || input.relay_id.is_empty() || input.relay_id.len() > 128 {
        return Ok(Json(json!({"ok": false, "reason": "malformed"})));
    }
    match db.redeem(&input.ticket, &input.relay_id).await {
        Ok(true) => {
            log::info!("event=relay_ticket_redeemed relay={} via=http", input.relay_id);
            Ok(Json(json!({"ok": true})))
        }
        Ok(false) => {
            let reason = db
                .ticket_failure(&input.ticket, &input.relay_id)
                .await
                .unwrap_or("unknown");
            log::warn!("event=relay_ticket_rejected relay={} via=http reason={reason}", input.relay_id);
            Ok(Json(json!({"ok": false, "reason": reason})))
        }
        Err(err) => {
            log::error!("event=relay_ticket_error relay={} via=http err={err}", input.relay_id);
            Err(error(StatusCode::INTERNAL_SERVER_ERROR, "Ticket store unavailable"))
        }
    }
}
