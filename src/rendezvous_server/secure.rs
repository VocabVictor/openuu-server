use super::*;
use hbb_common::{
    sodiumoxide::crypto::{box_, secretbox},
    tcp::Encrypt,
};

/// The server side of the client's `secure_tcp` handshake.
///
/// On a new TCP connection the server sends `KeyExchange { keys: [sign(our_pk)] }`, signed with
/// the rendezvous private key the client already trusts (`key` option / `RS_PUB_KEY`). The client
/// verifies the signature, generates a symmetric key, seals it to `our_pk` with a fresh box key
/// pair and answers `KeyExchange { keys: [their_pk, sealed_key] }`. Everything after that is
/// encrypted with the symmetric key by `hbb_common::tcp::Encrypt` on both ends. Clients that do
/// not expect the handshake skip the unsolicited `KeyExchange` and keep talking in clear text.
pub(super) struct KeyOffer {
    sk: box_::SecretKey,
}

impl KeyOffer {
    /// Builds the message to send first on the connection and keeps the box secret key needed to
    /// open the client's answer.
    pub(super) fn new(sk: &sign::SecretKey) -> (Self, RendezvousMessage) {
        let (pk, sk_b) = box_::gen_keypair();
        let mut msg = RendezvousMessage::new();
        msg.set_key_exchange(KeyExchange {
            keys: vec![sign::sign(&pk.0, sk).into()],
            ..Default::default()
        });
        (Self { sk: sk_b }, msg)
    }

    /// Opens the client's `KeyExchange` answer. Returns the symmetric key when the message is
    /// well formed and was sealed to our public key; `None` otherwise.
    pub(super) fn accept(&self, ex: &KeyExchange) -> Option<secretbox::Key> {
        if ex.keys.len() != 2 {
            return None;
        }
        let their_pk = box_::PublicKey::from_slice(ex.keys[0].as_ref())?;
        let nonce = box_::Nonce([0u8; box_::NONCEBYTES]);
        let key = box_::open(ex.keys[1].as_ref(), &nonce, &their_pk, &self.sk).ok()?;
        secretbox::Key::from_slice(&key)
    }
}

/// One `Encrypt` per direction: the sink and the read loop live in different tasks, and the
/// sequence counters of the two directions are independent anyway.
pub(super) fn encrypt_pair(key: &secretbox::Key) -> (Encrypt, Encrypt) {
    (Encrypt::new(key.clone()), Encrypt::new(key.clone()))
}
