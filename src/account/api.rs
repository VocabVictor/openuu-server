use super::*;

pub(super) async fn login(
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
pub(super) async fn current(Extension(db): Extension<Arc<Accounts>>, headers: HeaderMap) -> ApiResult {
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
pub(super) async fn logout(Extension(db): Extension<Arc<Accounts>>, headers: HeaderMap) -> ApiResult {
    db.revoke(bearer(&headers)).await.map_err(|_| {
        error(
            StatusCode::SERVICE_UNAVAILABLE,
            "Authentication unavailable",
        )
    })?;
    Ok(Json(json!({})))
}
pub(super) async fn empty_directory(Extension(db): Extension<Arc<Accounts>>, headers: HeaderMap) -> ApiResult {
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
pub(super) async fn address_book(Extension(db): Extension<Arc<Accounts>>, headers: HeaderMap) -> ApiResult {
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
pub(super) async fn relay_ticket(
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
