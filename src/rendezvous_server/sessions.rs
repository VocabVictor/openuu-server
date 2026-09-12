use super::*;

/// Controller sessions seen in a PunchHoleRequest, kept for a short while so that a
/// relay the controlled peer initiates afterwards (RelayResponse with a uuid of its own)
/// can still get a peer ticket minted from the controller's session
/// (docs/relay-ticket-peer-initiated.md). Keyed by the controller's full address and the
/// target id; no loose matching.
#[derive(Default)]
pub(super) struct PunchSessions(Mutex<HashMap<(SocketAddr, String), (String, Instant)>>);

const TTL: Duration = Duration::from_secs(60);

impl PunchSessions {
    pub(super) async fn remember(&self, controller: SocketAddr, target_id: &str, token: &str) {
        let mut map = self.0.lock().await;
        map.retain(|_, (_, at)| at.elapsed() < TTL);
        map.insert(
            (try_into_v4(controller), target_id.to_owned()),
            (token.to_owned(), Instant::now()),
        );
    }

    pub(super) async fn token_for(&self, controller: SocketAddr, target_id: &str) -> Option<String> {
        let map = self.0.lock().await;
        map.get(&(try_into_v4(controller), target_id.to_owned()))
            .filter(|(_, at)| at.elapsed() < TTL)
            .map(|(token, _)| token.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[hbb_common::tokio::test]
    async fn exact_key_and_expiry() {
        let sessions = PunchSessions::default();
        let a: SocketAddr = "198.51.100.2:4000".parse().unwrap();
        sessions.remember(a, "peer-1", "tok").await;
        assert_eq!(sessions.token_for(a, "peer-1").await.as_deref(), Some("tok"));
        let mapped: SocketAddr = "[::ffff:198.51.100.2]:4000".parse().unwrap();
        assert_eq!(sessions.token_for(mapped, "peer-1").await.as_deref(), Some("tok"));
        assert!(sessions.token_for("198.51.100.2:4001".parse().unwrap(), "peer-1").await.is_none());
        assert!(sessions.token_for(a, "peer-2").await.is_none());
        sessions.0.lock().await.get_mut(&(a, "peer-1".into())).unwrap().1 =
            Instant::now() - TTL - Duration::from_secs(1);
        assert!(sessions.token_for(a, "peer-1").await.is_none());
        sessions.remember(a, "peer-3", "tok3").await;
        assert!(!sessions.0.lock().await.contains_key(&(a, "peer-1".into())), "purged on insert");
    }
}
