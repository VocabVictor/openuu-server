//! `register_pk_result`, the transport-free half of RegisterPk shared by the UDP and TCP
//! paths: which requests are refused with UUID_MISMATCH or TOO_FREQUENT and which store
//! the key.

use super::secure_tests::test_server;
use super::*;
use hbb_common::rendezvous_proto::register_pk_response::Result as R;

fn rk(id: &str, uuid: &[u8], pk: &[u8]) -> RegisterPk {
    RegisterPk {
        id: id.to_owned(),
        uuid: uuid.to_vec().into(),
        pk: pk.to_vec().into(),
        ..Default::default()
    }
}

fn addr(ip: &str) -> SocketAddr {
    format!("{ip}:21116").parse().unwrap()
}

#[tokio::test]
async fn a_short_id_is_a_uuid_mismatch() {
    let mut rs = test_server(None).await;
    let res = rs.register_pk_result(rk("12345", b"u1", b"k1"), addr("198.51.100.7")).await;
    assert_eq!(res, R::UUID_MISMATCH);
}

#[tokio::test]
async fn the_first_registration_stores_uuid_and_pk() {
    let mut rs = test_server(None).await;
    let res = rs.register_pk_result(rk("100000001", b"u1", b"k1"), addr("198.51.100.7")).await;
    assert_eq!(res, R::OK);
    let peer = rs.pm.get_or("100000001").await;
    let peer = peer.read().await;
    assert_eq!(&peer.uuid[..], b"u1");
    assert_eq!(&peer.pk[..], b"k1");
    assert_eq!(peer.info.ip, "198.51.100.7");
}

#[tokio::test]
async fn another_uuid_for_a_known_id_is_refused() {
    let mut rs = test_server(None).await;
    assert_eq!(rs.register_pk_result(rk("100000002", b"u1", b"k1"), addr("198.51.100.7")).await, R::OK);
    let res = rs.register_pk_result(rk("100000002", b"u2", b"k1"), addr("198.51.100.7")).await;
    assert_eq!(res, R::UUID_MISMATCH);
    let peer = rs.pm.get_or("100000002").await;
    assert_eq!(&peer.read().await.uuid[..], b"u1", "the stored uuid is untouched");
}

#[tokio::test]
async fn a_new_ip_with_a_new_pk_is_refused_but_a_new_ip_alone_is_accepted() {
    let mut rs = test_server(None).await;
    assert_eq!(rs.register_pk_result(rk("100000003", b"u1", b"k1"), addr("198.51.100.7")).await, R::OK);
    let res = rs.register_pk_result(rk("100000003", b"u1", b"k2"), addr("198.51.100.8")).await;
    assert_eq!(res, R::UUID_MISMATCH);
    let res = rs.register_pk_result(rk("100000003", b"u1", b"k1"), addr("198.51.100.8")).await;
    assert_eq!(res, R::OK);
    let peer = rs.pm.get_or("100000003").await;
    assert_eq!(peer.read().await.info.ip, "198.51.100.8");
}

#[tokio::test]
async fn more_than_three_registrations_within_six_seconds_are_too_frequent() {
    let mut rs = test_server(None).await;
    let mut results = vec![];
    for _ in 0..4 {
        results.push(rs.register_pk_result(rk("100000004", b"u1", b"k1"), addr("198.51.100.7")).await);
    }
    assert_eq!(results, vec![R::OK, R::OK, R::OK, R::TOO_FREQUENT]);
}
