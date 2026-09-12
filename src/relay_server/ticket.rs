use hbb_common::{anyhow, bail, log, tokio::sync::OnceCell, ResultType};
use reqwest::Url;
use std::{net::IpAddr, time::Duration};

/// How this relay validates `RequestRelay.token` (docs/relay-ticket-http.md).
///
/// `Local` opens the account database in-process, as before. `Http` asks the
/// account service over its internal endpoint, which lets the relay run on a
/// host without the database; it is selected by `OPENUU_ACCOUNT_URL` plus
/// `OPENUU_INTERNAL_SECRET` (or `OPENUU_INTERNAL_SECRET_FILE`).
pub enum Redeemer {
    Local,
    Http {
        client: reqwest::Client,
        url: String,
        secret: String,
    },
}

const CONNECT_TIMEOUT: Duration = Duration::from_secs(2);
const TOTAL_TIMEOUT: Duration = Duration::from_secs(3);

static REDEEMER: OnceCell<Redeemer> = OnceCell::const_new();

pub fn init() -> ResultType<()> {
    let redeemer = Redeemer::from_env()?;
    if let Redeemer::Http { url, .. } = &redeemer {
        log::info!("event=relay_ticket_http url={url}");
    }
    REDEEMER.set(redeemer).ok();
    Ok(())
}

pub async fn redeem(ticket: &str, relay_id: &str) -> bool {
    match REDEEMER.get() {
        Some(r) => r.redeem(ticket, relay_id).await,
        None => Redeemer::Local.redeem(ticket, relay_id).await,
    }
}

impl Redeemer {
    pub fn from_env() -> ResultType<Self> {
        let url = match std::env::var("OPENUU_ACCOUNT_URL") {
            Ok(url) if !url.trim().is_empty() => url.trim().trim_end_matches('/').to_owned(),
            _ => return Ok(Self::Local),
        };
        let secret = match std::env::var("OPENUU_INTERNAL_SECRET_FILE") {
            Ok(path) => std::fs::read_to_string(&path)
                .map_err(|err| anyhow::anyhow!("cannot read OPENUU_INTERNAL_SECRET_FILE {path}: {err}"))?,
            Err(_) => std::env::var("OPENUU_INTERNAL_SECRET").unwrap_or_default(),
        };
        Self::http(&url, secret.trim())
    }

    pub fn http(url: &str, secret: &str) -> ResultType<Self> {
        check_url(url)?;
        if secret.len() < 16 {
            bail!("OPENUU_INTERNAL_SECRET must be set (16 bytes or more) when OPENUU_ACCOUNT_URL is set");
        }
        let client = reqwest::Client::builder()
            .no_proxy()
            .connect_timeout(CONNECT_TIMEOUT)
            .timeout(TOTAL_TIMEOUT)
            .redirect(reqwest::redirect::Policy::none())
            .build()?;
        Ok(Self::Http {
            client,
            url: format!("{}/api/internal/relay-ticket/redeem", url.trim_end_matches('/')),
            secret: secret.to_owned(),
        })
    }

    pub async fn redeem(&self, ticket: &str, relay_id: &str) -> bool {
        match self {
            Self::Local => hbbs::account::redeem_ticket(ticket, relay_id).await,
            Self::Http { client, url, secret } => {
                let body = serde_json::json!({"ticket": ticket, "relay_id": relay_id});
                // Retry once, and only when the first attempt never reached the server: a
                // request that got through may already have consumed the ticket.
                for attempt in 1..=2 {
                    let sent = client
                        .post(url)
                        .header("X-OpenUU-Internal", secret)
                        .json(&body)
                        .send()
                        .await;
                    match sent {
                        Err(err) if err.is_connect() && attempt == 1 => {
                            log::warn!("event=relay_ticket_http_retry relay={relay_id} err={err}");
                        }
                        Err(err) => {
                            log::error!("event=relay_ticket_http_error relay={relay_id} err={err}");
                            return false;
                        }
                        Ok(response) => return accept(response, relay_id).await,
                    }
                }
                false
            }
        }
    }
}

async fn accept(response: reqwest::Response, relay_id: &str) -> bool {
    let status = response.status();
    if !status.is_success() {
        log::error!("event=relay_ticket_http_error relay={relay_id} status={status}");
        return false;
    }
    match response.json::<serde_json::Value>().await {
        Ok(body) if body.get("ok").and_then(|v| v.as_bool()) == Some(true) => true,
        Ok(body) => {
            let reason = body.get("reason").and_then(|v| v.as_str()).unwrap_or("?");
            log::warn!("event=relay_ticket_rejected relay={relay_id} reason={reason}");
            false
        }
        Err(err) => {
            log::error!("event=relay_ticket_http_error relay={relay_id} err=malformed body: {err}");
            false
        }
    }
}

/// The secret must not cross the public internet in clear text: `https` to anything, or
/// `http` to a loopback or private *address*. Host names are only accepted with `https`.
pub fn check_url(url: &str) -> ResultType<()> {
    let parsed = Url::parse(url)?;
    match parsed.scheme() {
        "https" => Ok(()),
        "http" => {
            let host = parsed.host_str().unwrap_or_default();
            let Ok(ip) = host.trim_matches(|c| c == '[' || c == ']').parse::<IpAddr>() else {
                bail!("OPENUU_ACCOUNT_URL: plain http is only allowed to a loopback or private IP address, use https for a host name");
            };
            if is_private(ip) {
                Ok(())
            } else {
                bail!("OPENUU_ACCOUNT_URL: plain http to the public address {ip} is not allowed, use https")
            }
        }
        other => bail!("OPENUU_ACCOUNT_URL: unsupported scheme {other}"),
    }
}

fn is_private(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => v4.is_loopback() || v4.is_private(),
        IpAddr::V6(v6) => v6.is_loopback() || (v6.segments()[0] & 0xfe00) == 0xfc00,
    }
}
