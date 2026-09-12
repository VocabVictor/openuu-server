use super::*;

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
    pub(crate) async fn login(&self, name: String, password: String) -> ResultType<Option<String>> {
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
    /// Why `redeem` returned false, for the internal HTTP answer. Only reached on the
    /// failure path, so the extra lookup costs nothing on the normal one.
    pub async fn ticket_failure(&self, ticket: &str, relay_id: &str) -> ResultType<&'static str> {
        let row = sqlx::query("SELECT relay_id, expires FROM relay_tickets WHERE hash=?")
            .bind(digest(ticket))
            .fetch_optional(&self.pool)
            .await?;
        Ok(match row {
            None => "unknown",
            Some(row) if row.get::<i64, _>(1) <= now() => "expired",
            Some(row) if row.get::<String, _>(0) != relay_id => "relay_mismatch",
            Some(_) => "unknown",
        })
    }
    pub async fn revoke(&self, token: &str) -> ResultType<()> {
        sqlx::query("DELETE FROM account_sessions WHERE token_hash=?")
            .bind(digest(token))
            .execute(&self.pool)
            .await?;
        Ok(())
    }
}
