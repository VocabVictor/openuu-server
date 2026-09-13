use super::*;
use hbb_common::{
    sodiumoxide::crypto::{box_, secretbox},
    tcp::FramedStream,
    tokio::net::TcpListener,
};

fn answer(offer_msg: &RendezvousMessage, pk: &sign::PublicKey) -> (RendezvousMessage, secretbox::Key) {
    let Some(rendezvous_message::Union::KeyExchange(ex)) = &offer_msg.union else {
        panic!("first message is not a key exchange");
    };
    assert_eq!(ex.keys.len(), 1);
    let their_pk = sign::verify(&ex.keys[0], pk).expect("offer is signed with the server key");
    let their_pk = box_::PublicKey::from_slice(&their_pk).unwrap();
    let key = secretbox::gen_key();
    let (our_pk, our_sk) = box_::gen_keypair();
    let nonce = box_::Nonce([0u8; box_::NONCEBYTES]);
    let sealed = box_::seal(&key.0, &nonce, &their_pk, &our_sk);
    let mut msg = RendezvousMessage::new();
    msg.set_key_exchange(KeyExchange {
        keys: vec![our_pk.0.to_vec().into(), sealed.into()],
        ..Default::default()
    });
    (msg, key)
}

#[test]
fn accept_opens_a_key_sealed_to_the_offer() {
    let (pk, sk) = sign::gen_keypair();
    let (offer, msg) = secure::KeyOffer::new(&sk);
    let (reply, key) = answer(&msg, &pk);
    let Some(rendezvous_message::Union::KeyExchange(ex)) = reply.union else {
        unreachable!()
    };
    assert_eq!(offer.accept(&ex), Some(key));
}

#[test]
fn accept_rejects_a_key_sealed_to_another_offer() {
    let (pk, sk) = sign::gen_keypair();
    let (offer, _) = secure::KeyOffer::new(&sk);
    let (_, other_msg) = secure::KeyOffer::new(&sk);
    let (reply, _) = answer(&other_msg, &pk);
    let Some(rendezvous_message::Union::KeyExchange(ex)) = reply.union else {
        unreachable!()
    };
    assert_eq!(offer.accept(&ex), None);
}

#[test]
fn accept_rejects_malformed_answers() {
    let (_, sk) = sign::gen_keypair();
    let (offer, _) = secure::KeyOffer::new(&sk);
    let one_key = KeyExchange {
        keys: vec![vec![1u8; box_::PUBLICKEYBYTES].into()],
        ..Default::default()
    };
    assert_eq!(offer.accept(&one_key), None);
    let short_pk = KeyExchange {
        keys: vec![vec![1u8; 3].into(), vec![2u8; 48].into()],
        ..Default::default()
    };
    assert_eq!(offer.accept(&short_pk), None);
}

#[test]
fn offer_is_rejected_by_a_client_with_another_public_key() {
    let (_, sk) = sign::gen_keypair();
    let (other_pk, _) = sign::gen_keypair();
    let (_, msg) = secure::KeyOffer::new(&sk);
    let Some(rendezvous_message::Union::KeyExchange(ex)) = msg.union else {
        unreachable!()
    };
    assert!(sign::verify(&ex.keys[0], &other_pk).is_err());
}

pub(super) async fn test_server(sk: Option<sign::SecretKey>) -> RendezvousServer {
    let db = std::env::temp_dir().join(format!("hbbs-secure-test-{}.sqlite3", uuid::Uuid::new_v4()));
    std::env::set_var("DB_URL", &db);
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
            sk,
        }),
    }
}

async fn connect(rs: RendezvousServer) -> FramedStream {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let (stream, peer) = listener.accept().await.unwrap();
        let mut rs = rs;
        rs.handle_listener_inner(stream, peer, "", false).await.ok();
    });
    FramedStream::new(addr, None, 3_000).await.unwrap()
}

fn test_nat_request() -> RendezvousMessage {
    let mut msg = RendezvousMessage::new();
    msg.set_test_nat_request(TestNatRequest::new());
    msg
}

async fn next_msg(stream: &mut FramedStream) -> Option<RendezvousMessage> {
    let bytes = stream.next_timeout(3_000).await?.ok()?;
    RendezvousMessage::parse_from_bytes(&bytes).ok()
}

fn is_nat_response(msg: &RendezvousMessage) -> bool {
    matches!(msg.union, Some(rendezvous_message::Union::TestNatResponse(_)))
}

#[hbb_common::tokio::test]
async fn secure_client_talks_through_the_encrypted_channel() {
    let (pk, sk) = sign::gen_keypair();
    let mut stream = connect(test_server(Some(sk)).await).await;
    let offer = next_msg(&mut stream).await.expect("server sends the key offer first");
    let (reply, key) = answer(&offer, &pk);
    stream.send(&reply).await.unwrap();
    stream.set_key(key);
    stream.send(&test_nat_request()).await.unwrap();
    let res = next_msg(&mut stream).await.expect("encrypted reply");
    assert!(is_nat_response(&res), "{res:?}");
}

#[hbb_common::tokio::test]
async fn legacy_client_keeps_talking_in_clear_text() {
    let (_, sk) = sign::gen_keypair();
    let mut stream = connect(test_server(Some(sk)).await).await;
    let offer = next_msg(&mut stream).await.unwrap();
    assert!(matches!(offer.union, Some(rendezvous_message::Union::KeyExchange(_))));
    stream.send(&test_nat_request()).await.unwrap();
    let res = next_msg(&mut stream).await.expect("clear-text reply");
    assert!(is_nat_response(&res), "{res:?}");
}

#[hbb_common::tokio::test]
async fn server_without_key_sends_no_offer() {
    let mut stream = connect(test_server(None).await).await;
    stream.send(&test_nat_request()).await.unwrap();
    let res = next_msg(&mut stream).await.expect("clear-text reply");
    assert!(is_nat_response(&res), "{res:?}");
}

#[hbb_common::tokio::test]
async fn bad_key_exchange_closes_the_connection() {
    let (pk, sk) = sign::gen_keypair();
    let sk_copy = sk.clone();
    let mut stream = connect(test_server(Some(sk)).await).await;
    let (_, other_offer) = secure::KeyOffer::new(&sk_copy);
    next_msg(&mut stream).await.expect("server sends the key offer first");
    let (reply, _) = answer(&other_offer, &pk);
    stream.send(&reply).await.unwrap();
    assert!(next_msg(&mut stream).await.is_none());
}
