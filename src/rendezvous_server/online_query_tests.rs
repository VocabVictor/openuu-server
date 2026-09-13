//! The batch online query the user interface uses to light up its device list.
//!
//! It arrives on the extra TCP port (rendezvous port minus one), never over UDP, and it is
//! answered from `last_reg_time` alone. Two things make it worth its own tests: the answer
//! is a packed bitmap, where an off-by-one silently reports the wrong device rather than
//! failing; and a peer that registers over TCP must count as online exactly as a UDP one
//! does, which is what a peer running with UDP disabled depends on.
use super::*;
use hbb_common::{tcp::FramedStream, tokio::net::TcpListener};

async fn test_server() -> RendezvousServer {
    std::env::set_var(
        "DB_URL",
        std::env::temp_dir().join(format!("hbbs-online-{}.sqlite3", uuid::Uuid::new_v4())),
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

/// A loopback pair for the query itself: hbbs answers on the half it is handed.
async fn query_socket() -> (FramedStream, FramedStream) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let client = FramedStream::new(addr, None, 1000).await.unwrap();
    let (server_side, from) = listener.accept().await.unwrap();
    (FramedStream::from(server_side, from), client)
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

fn expired() -> Instant {
    Instant::now()
        .checked_sub(std::time::Duration::from_millis(REG_TIMEOUT as u64 * 2))
        .expect("uptime longer than a minute")
}

/// Runs the query and returns one flag per id, in the order asked.
async fn ask(rs: &mut RendezvousServer, ids: &[&str]) -> Vec<bool> {
    let (mut server_side, mut client) = query_socket().await;
    rs.handle_online_request(&mut server_side, ids.iter().map(|s| s.to_string()).collect())
        .await
        .expect("the query must be answered");
    let bytes = client
        .next_timeout(1000)
        .await
        .expect("no answer to the online query")
        .expect("the answer did not decode");
    match RendezvousMessage::parse_from_bytes(&bytes).unwrap().union {
        Some(rendezvous_message::Union::OnlineResponse(r)) => (0..ids.len())
            .map(|i| r.states[i / 8] & (0x01 << (7 - i % 8)) != 0)
            .collect(),
        other => panic!("expected OnlineResponse, got {other:?}"),
    }
}

/// The case a peer with UDP disabled depends on: it never sends a UDP registration, so if
/// the query only counted those, the device would sit in the list as permanently offline
/// while being perfectly reachable.
#[hbb_common::tokio::test]
async fn a_peer_that_registered_over_tcp_is_reported_online() {
    let mut rs = test_server().await;
    let (mut sink, mut client, peer_addr) = peer_socket().await;
    rs.handle_tcp(&register_pk("123456789"), &mut sink, peer_addr, "", false)
        .await;
    client.next_timeout(1000).await;

    assert_eq!(ask(&mut rs, &["123456789"]).await, vec![true]);
}

#[hbb_common::tokio::test]
async fn a_peer_that_stopped_registering_is_reported_offline() {
    let mut rs = test_server().await;
    let (mut sink, mut client, peer_addr) = peer_socket().await;
    rs.handle_tcp(&register_pk("123456789"), &mut sink, peer_addr, "", false)
        .await;
    client.next_timeout(1000).await;
    rs.pm
        .get_in_memory("123456789")
        .await
        .unwrap()
        .write()
        .await
        .last_reg_time = expired();

    assert_eq!(ask(&mut rs, &["123456789"]).await, vec![false]);
}

#[hbb_common::tokio::test]
async fn an_id_the_server_never_saw_is_reported_offline() {
    let mut rs = test_server().await;
    assert_eq!(ask(&mut rs, &["987654321"]).await, vec![false]);
}

/// The answer is a bitmap, so a peer is identified by its position rather than by its id.
/// This asks for more than eight ids with the only online one last, which is the case that
/// tells a correct packing apart from one that is a byte or a bit out: every wrong variant
/// reports some other device as the online one.
#[hbb_common::tokio::test]
async fn the_bits_follow_the_order_the_ids_were_asked_in() {
    let mut rs = test_server().await;
    let (mut sink, mut client, peer_addr) = peer_socket().await;
    rs.handle_tcp(&register_pk("123456789"), &mut sink, peer_addr, "", false)
        .await;
    client.next_timeout(1000).await;

    let ids = [
        "111111111",
        "222222222",
        "333333333",
        "444444444",
        "555555555",
        "666666666",
        "777777777",
        "888888888",
        "999999999",
        "123456789",
    ];
    let states = ask(&mut rs, &ids).await;
    assert_eq!(
        states,
        vec![false, false, false, false, false, false, false, false, false, true],
        "only the registered peer, and only in its own position"
    );
}
