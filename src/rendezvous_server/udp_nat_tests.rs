//! The UDP NAT test: a client sends TestNatRequest on the socket it will punch with and the
//! server answers with the public port it saw, so the client learns its own mapping.

use super::secure_tests::test_server;
use super::*;
use hbb_common::tokio::net::UdpSocket;

async fn nat_test_round(serial: i32) -> (RendezvousMessage, u16) {
    let mut rs = test_server(None).await;
    if serial > 0 {
        let mut inner: Inner = (*rs.inner).clone();
        inner.serial = serial;
        rs.inner = Arc::new(inner);
        rs.rendezvous_servers = Arc::new(vec!["rs.example.com".to_owned()]);
    }
    let mut server_socket = FramedSocket::new("127.0.0.1:0").await.unwrap();
    let server_addr = server_socket.local_addr().unwrap();
    let client = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let client_port = client.local_addr().unwrap().port();

    let mut msg = RendezvousMessage::new();
    msg.set_test_nat_request(TestNatRequest::default());
    client
        .send_to(&msg.write_to_bytes().unwrap(), server_addr)
        .await
        .unwrap();

    let (bytes, from) = match timeout(3_000, server_socket.next()).await {
        Ok(Some(Ok((bytes, from)))) => (bytes, from),
        other => panic!("no request on the server socket: {other:?}"),
    };
    rs.handle_udp(&bytes, from.into(), &mut server_socket, "").await.unwrap();

    let mut buf = [0u8; 1500];
    let (n, _) = timeout(3_000, client.recv_from(&mut buf))
        .await
        .expect("a reply within 3 s")
        .unwrap();
    (RendezvousMessage::parse_from_bytes(&buf[..n]).unwrap(), client_port)
}

#[hbb_common::tokio::test]
async fn a_udp_nat_test_is_answered_with_the_observed_port() {
    let (reply, client_port) = nat_test_round(0).await;
    match reply.union {
        Some(rendezvous_message::Union::TestNatResponse(res)) => {
            assert_eq!(res.port, client_port as i32);
            assert!(res.cu.is_none());
        }
        other => panic!("expected TestNatResponse, got {other:?}"),
    }
}

#[hbb_common::tokio::test]
async fn a_stale_client_serial_gets_the_server_list_with_the_answer() {
    let (reply, _) = nat_test_round(7).await;
    match reply.union {
        Some(rendezvous_message::Union::TestNatResponse(res)) => {
            let cu = res.cu.as_ref().expect("a ConfigUpdate");
            assert_eq!(cu.serial, 7);
            assert_eq!(cu.rendezvous_servers, vec!["rs.example.com".to_owned()]);
        }
        other => panic!("expected TestNatResponse, got {other:?}"),
    }
}
