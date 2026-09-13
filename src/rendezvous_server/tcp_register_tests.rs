//! TCP registration behaves like UDP registration: the peer is stored, heartbeats
//! refresh it, pushes reach it through its sink, and it times out the same way.
use super::*;
use hbb_common::{tcp::FramedStream, tokio::net::TcpListener};

async fn test_server() -> RendezvousServer {
    std::env::set_var(
        "DB_URL",
        std::env::temp_dir().join(format!("hbbs-tcpreg-{}.sqlite3", uuid::Uuid::new_v4())),
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

/// A loopback TCP pair: the server half wrapped as the peer's `Sink`, the client half to
/// read what hbbs writes back.
async fn peer_socket() -> (Option<Sink>, FramedStream, SocketAddr) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let client = FramedStream::new(addr, None, 1000).await.unwrap();
    let (server_side, peer_addr) = listener.accept().await.unwrap();
    let (a, _b) = Framed::new(server_side, BytesCodec::new()).split();
    (Some(Sink::TcpStream(a, None)), client, peer_addr)
}

async fn reply(client: &mut FramedStream) -> Option<rendezvous_message::Union> {
    let bytes = client.next_timeout(1000).await?.ok()?;
    RendezvousMessage::parse_from_bytes(&bytes).ok()?.union
}

/// An instant older than REG_TIMEOUT. `get_expired_time` cannot be used here: on a
/// freshly booted machine `Instant::checked_sub(1h)` is None and it returns "now".
fn expired() -> Instant {
    Instant::now()
        .checked_sub(std::time::Duration::from_millis(REG_TIMEOUT as u64 * 2))
        .expect("uptime longer than a minute")
}

fn register_pk(id: &str) -> Vec<u8> {
    let mut msg = RendezvousMessage::new();
    msg.set_register_pk(RegisterPk {
        id: id.into(),
        uuid: "uuid-1".into(),
        pk: "pk-1".into(),
        ..Default::default()
    });
    msg.write_to_bytes().unwrap()
}

fn register_peer(id: &str) -> Vec<u8> {
    let mut msg = RendezvousMessage::new();
    msg.set_register_peer(RegisterPeer {
        id: id.into(),
        ..Default::default()
    });
    msg.write_to_bytes().unwrap()
}

#[hbb_common::tokio::test]
async fn register_pk_over_tcp_stores_the_peer_and_answers_ok() {
    let mut rs = test_server().await;
    let (mut sink, mut client, peer_addr) = peer_socket().await;
    rs.handle_tcp(&register_pk("123456789"), &mut sink, peer_addr, "", false).await;
    match reply(&mut client).await {
        Some(rendezvous_message::Union::RegisterPkResponse(r)) => {
            assert_eq!(r.result.enum_value_or_default(), register_pk_response::Result::OK)
        }
        other => panic!("expected RegisterPkResponse, got {other:?}"),
    }
    assert!(sink.is_none(), "the connection sink is adopted by tcp_peers");
    let peer = rs.pm.get_in_memory("123456789").await.expect("peer stored");
    assert_eq!(peer.read().await.socket_addr, peer_addr);
    assert!(rs.tcp_peers.lock().await.contains_key(&try_into_v4(peer_addr)));
}

#[hbb_common::tokio::test]
async fn register_peer_over_tcp_refreshes_the_peer_and_replies() {
    let mut rs = test_server().await;
    let (mut sink, mut client, peer_addr) = peer_socket().await;
    rs.handle_tcp(&register_pk("123456789"), &mut sink, peer_addr, "", false).await;
    reply(&mut client).await;
    {
        let peer = rs.pm.get_in_memory("123456789").await.unwrap();
        peer.write().await.last_reg_time = expired();
    }
    rs.handle_tcp(&register_peer("123456789"), &mut sink, peer_addr, "", false).await;
    match reply(&mut client).await {
        Some(rendezvous_message::Union::RegisterPeerResponse(r)) => assert!(!r.request_pk),
        other => panic!("expected RegisterPeerResponse, got {other:?}"),
    }
    let peer = rs.pm.get_in_memory("123456789").await.unwrap();
    assert!(peer.read().await.last_reg_time.elapsed().as_millis() < REG_TIMEOUT as u128);
}

#[hbb_common::tokio::test]
async fn pushes_reach_a_tcp_registered_peer_through_its_sink() {
    let mut rs = test_server().await;
    let (mut sink, mut client, peer_addr) = peer_socket().await;
    rs.handle_tcp(&register_pk("123456789"), &mut sink, peer_addr, "", false).await;
    reply(&mut client).await;
    let mut msg = RendezvousMessage::new();
    msg.set_punch_hole(PunchHole {
        relay_server: "relay.example".into(),
        ..Default::default()
    });
    assert!(rs.send_to_tcp_peer(peer_addr, msg.clone()).await);
    match reply(&mut client).await {
        Some(rendezvous_message::Union::PunchHole(ph)) => {
            assert_eq!(ph.relay_server, "relay.example")
        }
        other => panic!("expected PunchHole, got {other:?}"),
    }
    // the sink survives a send and keeps working
    assert!(rs.send_to_tcp_peer(peer_addr, msg).await);
    assert!(reply(&mut client).await.is_some());
    let stranger: SocketAddr = "198.51.100.7:5000".parse().unwrap();
    assert!(
        !rs.send_to_tcp_peer(stranger, RendezvousMessage::new()).await,
        "unknown address falls back to UDP"
    );
}

#[hbb_common::tokio::test]
async fn a_tcp_registered_peer_goes_offline_after_reg_timeout_like_udp() {
    let mut rs = test_server().await;
    let (mut sink, mut client, peer_addr) = peer_socket().await;
    rs.handle_tcp(&register_pk("123456789"), &mut sink, peer_addr, "", false).await;
    reply(&mut client).await;
    let controller: SocketAddr = "198.51.100.2:4000".parse().unwrap();
    let request = PunchHoleRequest {
        id: "123456789".into(),
        ..Default::default()
    };
    let (_, to) = rs
        .handle_punch_hole_request(controller, request.clone(), "", false)
        .await
        .unwrap();
    assert_eq!(to, Some(peer_addr), "fresh registration: punch goes to the peer");
    rs.pm
        .get_in_memory("123456789")
        .await
        .unwrap()
        .write()
        .await
        .last_reg_time = expired();
    let (msg, to) = rs
        .handle_punch_hole_request(controller, request, "", false)
        .await
        .unwrap();
    assert_eq!(to, None);
    match msg.union {
        Some(rendezvous_message::Union::PunchHoleResponse(r)) => assert_eq!(
            r.failure.enum_value_or_default(),
            punch_hole_response::Failure::OFFLINE
        ),
        other => panic!("expected OFFLINE, got {other:?}"),
    }
}

#[hbb_common::tokio::test]
async fn closing_the_connection_forgets_the_sink() {
    let mut rs = test_server().await;
    let (mut sink, mut client, peer_addr) = peer_socket().await;
    rs.handle_tcp(&register_pk("123456789"), &mut sink, peer_addr, "", false).await;
    reply(&mut client).await;
    rs.forget_tcp_peer(peer_addr).await;
    assert!(!rs.send_to_tcp_peer(peer_addr, RendezvousMessage::new()).await);
}
