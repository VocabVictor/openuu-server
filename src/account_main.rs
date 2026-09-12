use flexi_logger::{opt_format, Logger, WriteMode};
use hbb_common::{bail, log, tokio, ResultType};
use hbbs::account::Accounts;
use std::sync::Arc;

const USAGE: &str = "Usage:
    openuu-account                      Run the account HTTP service
    openuu-account create NAME          Create an account (password from OPENUU_NEW_PASSWORD)
    openuu-account passwd NAME          Replace the password (from OPENUU_NEW_PASSWORD), revokes sessions
    openuu-account disable NAME         Disable an account and revoke its sessions
    openuu-account enable NAME          Re-enable an account
    openuu-account delete NAME          Delete an account, its sessions, tickets and wake state
    openuu-account list                 List accounts with enabled flag and live session count

Environment: OPENUU_ACCOUNT_DB (default openuu-accounts.sqlite3), OPENUU_ACCOUNT_BIND (default 127.0.0.1:21114), OPENUU_PEER_DB (hbbs db_v2.sqlite3, default ./db_v2.sqlite3; audit records from ids not in it are dropped)";

fn new_password() -> ResultType<String> {
    let password = std::env::var("OPENUU_NEW_PASSWORD")
        .map_err(|_| hbb_common::anyhow::anyhow!("Set OPENUU_NEW_PASSWORD in the environment"))?;
    std::env::remove_var("OPENUU_NEW_PASSWORD");
    Ok(password)
}

async fn admin(db: &Accounts, command: &str, name: Option<&str>) -> ResultType<()> {
    let name = match (command, name) {
        ("list", None) => "",
        ("list", Some(_)) | (_, None) => bail!("{USAGE}"),
        (_, Some(name)) => name,
    };
    match command {
        "create" => {
            db.create_user(name, new_password()?).await?;
            println!("Account created");
        }
        "passwd" => {
            db.set_password(name, new_password()?).await?;
            println!("Password changed; existing sessions revoked");
        }
        "disable" => {
            db.set_enabled(name, false).await?;
            println!("Account disabled; existing sessions revoked");
        }
        "enable" => {
            db.set_enabled(name, true).await?;
            println!("Account enabled");
        }
        "delete" => {
            db.delete_user(name).await?;
            println!("Account deleted");
        }
        "list" => {
            println!("{:<32} {:<8} {}", "NAME", "ENABLED", "SESSIONS");
            for a in db.list_users().await? {
                println!("{:<32} {:<8} {}", a.name, a.enabled, a.sessions);
            }
        }
        _ => bail!("{USAGE}"),
    }
    Ok(())
}

#[tokio::main]
async fn main() -> ResultType<()> {
    let _logger = Logger::try_with_env_or_str("info")?
        .log_to_stdout()
        .format(opt_format)
        .write_mode(WriteMode::Direct)
        .start()?;
    let args: Vec<String> = std::env::args().skip(1).collect();
    if matches!(args.first().map(String::as_str), Some("-h" | "--help" | "help")) {
        println!("{USAGE}");
        return Ok(());
    }
    let path =
        std::env::var("OPENUU_ACCOUNT_DB").unwrap_or_else(|_| "openuu-accounts.sqlite3".into());
    let peer_db = std::env::var("OPENUU_PEER_DB").unwrap_or_else(|_| "db_v2.sqlite3".into());
    let db = Arc::new(match Accounts::open_with_peers(&path, Some(&peer_db)).await {
        Ok(db) => db,
        Err(err) => {
            log::warn!("event=peer_db_unavailable path={peer_db} err={err}; connection audits will be dropped");
            Accounts::open(&path).await?
        }
    });
    if let Some(command) = args.first() {
        if args.len() > 2 {
            bail!("{USAGE}");
        }
        return admin(&db, command, args.get(1).map(String::as_str)).await;
    }
    let addr = std::env::var("OPENUU_ACCOUNT_BIND")
        .unwrap_or_else(|_| "127.0.0.1:21114".into())
        .parse()?;
    log::info!("event=account_service_start bind={addr} db={path}");
    axum::Server::bind(&addr)
        .serve(
            hbbs::account::router(db).into_make_service_with_connect_info::<std::net::SocketAddr>(),
        )
        .await?;
    Ok(())
}
