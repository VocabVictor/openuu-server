use hbb_common::{tokio, ResultType};
use std::sync::Arc;
#[tokio::main]
async fn main() -> ResultType<()> {
    let path =
        std::env::var("OPENUU_ACCOUNT_DB").unwrap_or_else(|_| "openuu-accounts.sqlite3".into());
    let db = Arc::new(hbbs::account::Accounts::open(&path).await?);
    if let Some(name) = std::env::args().nth(1) {
        let password = std::env::var("OPENUU_NEW_PASSWORD")?;
        std::env::remove_var("OPENUU_NEW_PASSWORD");
        db.create_user(&name, password).await?;
        println!("Account created");
        return Ok(());
    }
    let addr = std::env::var("OPENUU_ACCOUNT_BIND")
        .unwrap_or_else(|_| "127.0.0.1:21114".into())
        .parse()?;
    axum::Server::bind(&addr)
        .serve(
            hbbs::account::router(db).into_make_service_with_connect_info::<std::net::SocketAddr>(),
        )
        .await?;
    Ok(())
}
