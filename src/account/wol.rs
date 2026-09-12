use super::*;

pub(super) async fn init(pool: &SqlitePool) -> ResultType<()> {
    sqlx::query("CREATE TABLE IF NOT EXISTS wol_devices(owner TEXT NOT NULL,id TEXT NOT NULL,seen INTEGER NOT NULL,session BLOB NOT NULL,peers TEXT NOT NULL,PRIMARY KEY(owner,id))").execute(pool).await?;
    sqlx::query("CREATE TABLE IF NOT EXISTS wol_jobs(owner TEXT NOT NULL,target TEXT NOT NULL,helper TEXT NOT NULL,expires INTEGER NOT NULL,PRIMARY KEY(owner,target))").execute(pool).await?;
    Ok(())
}

#[derive(Deserialize)]
pub(super) struct Poll {
    id: String,
    peers: Vec<String>,
}
#[derive(Deserialize)]
pub(super) struct Target {
    id: String,
}
fn valid(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 128
        && id
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || b"._-".contains(&c))
}
fn unavailable(_: sqlx::Error) -> (StatusCode, Json<Value>) {
    error(StatusCode::SERVICE_UNAVAILABLE, "Wake service unavailable")
}
async fn owner(db: &Accounts, headers: &HeaderMap) -> Result<String, (StatusCode, Json<Value>)> {
    db.user(bearer(headers))
        .await
        .map_err(|_| {
            error(
                StatusCode::SERVICE_UNAVAILABLE,
                "Authentication unavailable",
            )
        })?
        .ok_or_else(|| error(StatusCode::UNAUTHORIZED, "Login required"))
}

pub(super) async fn poll(
    Extension(db): Extension<Arc<Accounts>>,
    headers: HeaderMap,
    Json(input): Json<Poll>,
) -> ApiResult {
    let owner = owner(&db, &headers).await?;
    if !valid(&input.id) || input.peers.len() > 256 || input.peers.iter().any(|p| !valid(p)) {
        return Err(error(StatusCode::BAD_REQUEST, "Invalid device list"));
    }
    let peers = serde_json::to_string(&input.peers)
        .map_err(|_| error(StatusCode::BAD_REQUEST, "Invalid device list"))?;
    sqlx::query("INSERT INTO wol_devices(owner,id,seen,session,peers) VALUES(?,?,?,?,?) ON CONFLICT(owner,id) DO UPDATE SET seen=excluded.seen,session=excluded.session,peers=excluded.peers")
        .bind(&owner).bind(&input.id).bind(now()).bind(digest(bearer(&headers))).bind(peers).execute(&db.pool).await.map_err(unavailable)?;
    // DELETE RETURNING leases each request once. The retained target cache is local to the helper.
    let jobs =
        sqlx::query("DELETE FROM wol_jobs WHERE owner=? AND helper=? RETURNING target,expires")
            .bind(&owner)
            .bind(&input.id)
            .fetch_all(&db.pool)
            .await
            .map_err(unavailable)?;
    let targets: Vec<String> = jobs
        .into_iter()
        .filter(|r| r.get::<i64, _>("expires") > now())
        .map(|r| r.get::<String, _>("target"))
        .filter(|id| input.peers.contains(id))
        .collect();
    Ok(Json(json!({"targets":targets})))
}

async fn availability(
    db: &Accounts,
    owner: &str,
    target: &str,
) -> Result<(String, Option<String>), (StatusCode, Json<Value>)> {
    let target_row = sqlx::query("SELECT seen FROM wol_devices WHERE owner=? AND id=?")
        .bind(owner)
        .bind(target)
        .fetch_optional(&db.pool)
        .await
        .map_err(unavailable)?;
    let Some(row) = target_row else {
        return Ok(("unregistered".into(), None));
    };
    if row.get::<i64, _>("seen") >= now() - 30 {
        return Ok(("online".into(), None));
    }
    let helpers = sqlx::query("SELECT d.id,d.peers FROM wol_devices d JOIN account_sessions s ON d.session=s.token_hash WHERE d.owner=? AND d.id<>? AND d.seen>=? AND s.expires>? AND s.name=d.owner ORDER BY d.seen DESC")
        .bind(owner).bind(target).bind(now()-30).bind(now()).fetch_all(&db.pool).await.map_err(unavailable)?;
    for row in helpers {
        let peers: Vec<String> =
            serde_json::from_str(&row.get::<String, _>("peers")).unwrap_or_default();
        if peers.iter().any(|p| p == target) {
            return Ok(("available".into(), Some(row.get("id"))));
        }
    }
    Ok(("no_helper".into(), None))
}

pub(super) async fn status(
    Extension(db): Extension<Arc<Accounts>>,
    headers: HeaderMap,
    Json(input): Json<Target>,
) -> ApiResult {
    let owner = owner(&db, &headers).await?;
    let (state, _) = availability(&db, &owner, &input.id).await?;
    Ok(Json(json!({"state":state})))
}

pub(super) async fn wake(
    Extension(db): Extension<Arc<Accounts>>,
    headers: HeaderMap,
    Json(input): Json<Target>,
) -> ApiResult {
    let owner = owner(&db, &headers).await?;
    let (state, helper) = availability(&db, &owner, &input.id).await?;
    let Some(helper) = helper else {
        return Err(error(StatusCode::CONFLICT, &state));
    };
    let result = sqlx::query("INSERT INTO wol_jobs(owner,target,helper,expires) VALUES(?,?,?,?) ON CONFLICT(owner,target) DO UPDATE SET helper=excluded.helper,expires=excluded.expires WHERE wol_jobs.expires<?")
        .bind(&owner).bind(&input.id).bind(helper).bind(now()+30).bind(now()).execute(&db.pool).await.map_err(unavailable)?;
    if result.rows_affected() == 0 {
        return Err(error(
            StatusCode::TOO_MANY_REQUESTS,
            "Wake request already queued",
        ));
    }
    Ok(Json(json!({"state":"queued"})))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn wake_requires_owner_offline_target_and_live_helper() {
        let db = Arc::new(Accounts::open(":memory:").await.unwrap());
        db.create_user("alice", "test-password-long".into())
            .await
            .unwrap();
        db.create_user("bob", "test-password-long".into())
            .await
            .unwrap();
        let token = db
            .login("alice".into(), "test-password-long".into())
            .await
            .unwrap()
            .unwrap();
        let mut h = HeaderMap::new();
        h.insert(
            "authorization",
            format!("Bearer {}", token).parse().unwrap(),
        );
        poll(
            Extension(db.clone()),
            h.clone(),
            Json(Poll {
                id: "target".into(),
                peers: vec![],
            }),
        )
        .await
        .unwrap();
        assert_eq!(
            availability(&db, "alice", "target").await.unwrap().0,
            "online"
        );
        sqlx::query("UPDATE wol_devices SET seen=0")
            .execute(&db.pool)
            .await
            .unwrap();
        assert_eq!(
            availability(&db, "alice", "target").await.unwrap().0,
            "no_helper"
        );
        poll(
            Extension(db.clone()),
            h.clone(),
            Json(Poll {
                id: "helper".into(),
                peers: vec!["target".into()],
            }),
        )
        .await
        .unwrap();
        assert_eq!(
            availability(&db, "bob", "target").await.unwrap().0,
            "unregistered"
        );
        assert!(wake(
            Extension(db.clone()),
            HeaderMap::new(),
            Json(Target {
                id: "target".into()
            })
        )
        .await
        .is_err());
        let bob = db
            .login("bob".into(), "test-password-long".into())
            .await
            .unwrap()
            .unwrap();
        let mut bh = HeaderMap::new();
        bh.insert("authorization", format!("Bearer {}", bob).parse().unwrap());
        assert!(wake(
            Extension(db.clone()),
            bh,
            Json(Target {
                id: "target".into()
            })
        )
        .await
        .is_err());
        wake(
            Extension(db.clone()),
            h.clone(),
            Json(Target {
                id: "target".into(),
            }),
        )
        .await
        .unwrap();
        assert!(wake(
            Extension(db.clone()),
            h.clone(),
            Json(Target {
                id: "target".into()
            })
        )
        .await
        .is_err());
        let jobs = poll(
            Extension(db.clone()),
            h.clone(),
            Json(Poll {
                id: "helper".into(),
                peers: vec!["target".into()],
            }),
        )
        .await
        .unwrap();
        assert_eq!(jobs.0["targets"], json!(["target"]));
        let jobs = poll(
            Extension(db.clone()),
            h.clone(),
            Json(Poll {
                id: "helper".into(),
                peers: vec!["target".into()],
            }),
        )
        .await
        .unwrap();
        assert_eq!(jobs.0["targets"], json!([]));
        db.revoke(&token).await.unwrap();
        assert!(poll(
            Extension(db),
            h,
            Json(Poll {
                id: "helper".into(),
                peers: vec![]
            })
        )
        .await
        .is_err());
    }
}
