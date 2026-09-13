use super::*;
use hbb_common::{tcp::FramedStream, tokio::net::TcpListener};

async fn test_server() -> RendezvousServer {
    std::env::set_var(
        "DB_URL",
        std::env::temp_dir().join(format!("hbbs-rr-{}.sqlite3", uuid::Uuid::new_v4())),
    );
    let (tx, _rx) = mpsc::unbounded_channel::<Data>();
    RendezvousServer {
        tcp_punch: Default::default(),
        tcp_peers: Default::default(),
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
    }
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

/// A loopback TCP pair: the server half wrapped as the peer's `Sink`, the client half to read
/// what hbbs writes back.
async fn peer_socket() -> (Option<Sink>, FramedStream, SocketAddr) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let client = FramedStream::new(addr, None, 1000).await.unwrap();
    let (server_side, peer_addr) = listener.accept().await.unwrap();
    let (a, _b) = Framed::new(server_side, BytesCodec::new()).split();
    (Some(Sink::TcpStream(a, None)), client, peer_addr)
}

fn relay_response(controller: SocketAddr, id: &str, uuid: &str) -> Vec<u8> {
    let mut msg = RendezvousMessage::new();
    let mut rr = RelayResponse {
        socket_addr: AddrMangle::encode(controller).into(),
        uuid: uuid.into(),
        relay_server: "relay.example".into(),
        ..Default::default()
    };
    rr.set_id(id.into());
    msg.set_relay_response(rr);
    msg.write_to_bytes().unwrap()
}

async fn reply(client: &mut FramedStream) -> Option<RequestRelay> {
    let bytes = client.next_timeout(1000).await?.ok()?;
    match RendezvousMessage::parse_from_bytes(&bytes).ok()?.union {
        Some(rendezvous_message::Union::RequestRelay(rf)) => Some(rf),
        _ => None,
    }
}

#[hbb_common::tokio::test]
async fn relay_response_after_a_punch_request_gets_a_peer_ticket() {
    let mut rs = test_server().await;
    let controller: SocketAddr = "198.51.100.2:4000".parse().unwrap();
    let token = session_token().await;
    rs.punch_sessions.remember(controller, "peer-1", &token).await;
    let (mut sink, mut client, peer_addr) = peer_socket().await;
    let bytes = relay_response(controller, "peer-1", "uuid-9");
    rs.handle_tcp(&bytes, &mut sink, peer_addr, "", false).await;
    let rf = reply(&mut client).await.expect("hbbs answers with a RequestRelay");
    assert_eq!(rf.uuid, "uuid-9");
    assert_eq!(rf.token.len(), 64);
    let db = crate::account::shared().await.unwrap();
    assert!(!db.redeem(&rf.token, "uuid-8").await.unwrap());
    assert!(db.redeem(&rf.token, "uuid-9").await.unwrap());
    assert!(!db.redeem(&rf.token, "uuid-9").await.unwrap());
}

#[hbb_common::tokio::test]
async fn relay_response_without_a_known_session_gets_nothing() {
    let mut rs = test_server().await;
    let controller: SocketAddr = "198.51.100.2:4000".parse().unwrap();
    let token = session_token().await;
    let other_port: SocketAddr = "198.51.100.2:4001".parse().unwrap();
    rs.punch_sessions.remember(other_port, "peer-1", &token).await;
    let (mut sink, mut client, peer_addr) = peer_socket().await;
    let bytes = relay_response(controller, "peer-1", "uuid-9");
    rs.handle_tcp(&bytes, &mut sink, peer_addr, "", false).await;
    assert!(reply(&mut client).await.is_none(), "other port is a miss");

    rs.punch_sessions.remember(controller, "peer-1", &token).await;
    let (mut sink, mut client, peer_addr) = peer_socket().await;
    let bytes = relay_response(controller, "peer-1", "");
    rs.handle_tcp(&bytes, &mut sink, peer_addr, "", false).await;
    assert!(reply(&mut client).await.is_none(), "no uuid, not peer-initiated");
}

#[hbb_common::tokio::test]
async fn punch_request_records_the_session() {
    let mut rs = test_server().await;
    let controller: SocketAddr = "198.51.100.2:4000".parse().unwrap();
    let token = session_token().await;
    let mut msg = RendezvousMessage::new();
    msg.set_punch_hole_request(PunchHoleRequest {
        id: "peer-1".into(),
        token: token.clone(),
        ..Default::default()
    });
    let mut sink = None;
    let bytes = msg.write_to_bytes().unwrap();
    rs.handle_tcp(&bytes, &mut sink, controller, "", false).await;
    assert_eq!(rs.punch_sessions.token_for(controller, "peer-1").await, Some(token));
}
