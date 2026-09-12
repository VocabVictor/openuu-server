use super::*;

async fn test_server() -> (RendezvousServer, mpsc::UnboundedReceiver<Data>) {
    let dir = std::env::temp_dir();
    std::env::set_var("DB_URL", dir.join(format!("hbbs-fwd-{}.sqlite3", uuid::Uuid::new_v4())));
    let (tx, rx) = mpsc::unbounded_channel::<Data>();
    let rs = RendezvousServer {
        tcp_punch: Default::default(),
        punch_sessions: Default::default(),
        pm: PeerMap::new().await.unwrap(),
        tx,
        relay_servers: Default::default(),
        relay_servers0: Default::default(),
        rendezvous_servers: Default::default(),
        inner: Arc::new(Inner {
            serial: 0,
            version: String::new(),
            software_url: String::new(),
            mask: None,
            local_ip: String::new(),
            sk: None,
        }),
    };
    (rs, rx)
}

async fn session_token() -> String {
    std::env::set_var(
        "OPENUU_ACCOUNT_DB",
        std::env::temp_dir().join(format!("hbbs-fwd-accounts-{}.sqlite3", std::process::id())),
    );
    let db = crate::account::shared().await.unwrap();
    db.create_user("controller", "test-only-long-password".into())
        .await
        .ok();
    db.login("controller".into(), "test-only-long-password".into())
        .await
        .unwrap()
        .unwrap()
}

fn request_relay(token: &str) -> Vec<u8> {
    let mut msg = RendezvousMessage::new();
    msg.set_request_relay(RequestRelay {
        id: "peer-1".into(),
        uuid: "uuid-1".into(),
        token: token.into(),
        relay_server: "relay.example:21117".into(),
        ..Default::default()
    });
    msg.write_to_bytes().unwrap()
}

#[hbb_common::tokio::test]
async fn forwarded_request_relay_carries_a_peer_ticket() {
    let (mut rs, mut rx) = test_server().await;
    let peer_addr: SocketAddr = "203.0.113.7:21116".parse().unwrap();
    rs.pm.get_or("peer-1").await.write().await.socket_addr = peer_addr;
    let controller: SocketAddr = "198.51.100.2:4000".parse().unwrap();
    let token = session_token().await;
    let mut sink = None;
    assert!(rs.handle_tcp(&request_relay(&token), &mut sink, controller, "", false).await);
    let Some(Data::Msg(msg, to)) = rx.recv().await else {
        panic!("nothing forwarded");
    };
    assert_eq!(to, peer_addr);
    let Some(rendezvous_message::Union::RequestRelay(rf)) = msg.union else {
        panic!("not a RequestRelay: {msg:?}");
    };
    assert_eq!(rf.uuid, "uuid-1");
    assert_eq!(rf.token.len(), 64, "peer ticket, not the session token");
    assert_ne!(rf.token, token);
    assert_eq!(AddrMangle::decode(&rf.socket_addr), controller);
    let db = crate::account::shared().await.unwrap();
    assert!(!db.redeem(&rf.token, "uuid-2").await.unwrap(), "bound to the uuid");
    assert!(db.redeem(&rf.token, "uuid-1").await.unwrap());
    assert!(!db.redeem(&rf.token, "uuid-1").await.unwrap(), "single use");
}

#[hbb_common::tokio::test]
async fn unauthorized_request_relay_is_dropped() {
    let (mut rs, mut rx) = test_server().await;
    rs.pm.get_or("peer-1").await.write().await.socket_addr = "203.0.113.7:21116".parse().unwrap();
    let _ = session_token().await;
    let mut sink = None;
    let from: SocketAddr = "198.51.100.2:4000".parse().unwrap();
    assert!(!rs.handle_tcp(&request_relay(&"b".repeat(64)), &mut sink, from, "", false).await);
    assert!(rx.try_recv().is_err());
}
