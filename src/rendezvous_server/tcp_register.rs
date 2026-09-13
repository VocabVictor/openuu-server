use super::*;

/// Peer registration over the (secured) TCP rendezvous connection. The UDP path in
/// `udp.rs` is unchanged; both share `register_peer_response` / `register_pk_result`.
/// A peer registered here keeps its connection's sink in `tcp_peers`, keyed by the
/// connection address that is also stored as the peer's `socket_addr`, so every
/// message hbbs would push to that address over UDP (`PunchHole`, `RequestRelay`,
/// `FetchLocalAddr`, ...) is delivered through the encrypted TCP stream instead.
impl RendezvousServer {
    /// `RegisterPeer` received on TCP: adopt the connection sink on first use, refresh the
    /// peer like the UDP path does and answer through the same connection.
    pub(super) async fn handle_tcp_register_peer(
        &mut self,
        rp: RegisterPeer,
        sink: &mut Option<Sink>,
        addr: SocketAddr,
    ) {
        if rp.id.is_empty() {
            return;
        }
        if let Some(s) = sink.take() {
            self.tcp_peers.lock().await.insert(try_into_v4(addr), s);
            log::info!("event=peer_register transport=tcp id={} from={}", rp.id, addr);
        }
        let msg_out = self.register_peer_response(rp.id, addr).await;
        self.send_to_tcp_peer(addr, msg_out).await;
        if self.inner.serial > rp.serial {
            let mut msg_out = RendezvousMessage::new();
            msg_out.set_configure_update(ConfigUpdate {
                serial: self.inner.serial,
                rendezvous_servers: (*self.rendezvous_servers).clone(),
                ..Default::default()
            });
            self.send_to_tcp_peer(addr, msg_out).await;
        }
    }

    /// `RegisterPk` received on TCP: same validation and storage as the UDP path, answer
    /// through the connection.
    pub(super) async fn handle_tcp_register_pk(
        &mut self,
        rk: RegisterPk,
        sink: &mut Option<Sink>,
        addr: SocketAddr,
    ) {
        if rk.uuid.is_empty() || rk.pk.is_empty() {
            return;
        }
        if let Some(s) = sink.take() {
            self.tcp_peers.lock().await.insert(try_into_v4(addr), s);
            log::info!("event=peer_register transport=tcp id={} from={} pk=true", rk.id, addr);
        }
        let res = self.register_pk_result(rk, addr).await;
        let mut msg_out = RendezvousMessage::new();
        msg_out.set_register_pk_response(RegisterPkResponse {
            result: res.into(),
            ..Default::default()
        });
        self.send_to_tcp_peer(addr, msg_out).await;
    }

    /// Sends to a TCP-registered peer without giving up its sink. Returns false when no
    /// such peer is connected, so the caller can fall back to UDP.
    pub(super) async fn send_to_tcp_peer(&self, addr: SocketAddr, msg: RendezvousMessage) -> bool {
        let mut peers = self.tcp_peers.lock().await;
        match peers.get_mut(&try_into_v4(addr)) {
            Some(sink) => {
                let mut slot = Some(std::mem::replace(sink, Sink::Closed));
                Self::send_to_sink(&mut slot, msg).await;
                if let Some(s) = slot {
                    *sink = s;
                }
                true
            }
            None => false,
        }
    }

    /// Delivers a message hbbs pushes to a peer address: through the peer's TCP sink when
    /// it registered over TCP, otherwise over UDP as before.
    pub(super) async fn deliver(&self, socket: &mut FramedSocket, msg: &RendezvousMessage, addr: SocketAddr) {
        if self.send_to_tcp_peer(addr, msg.clone()).await {
            return;
        }
        allow_err!(socket.send(msg, addr).await);
    }

    /// Forgets the TCP sink of a closed connection.
    pub(super) async fn forget_tcp_peer(&self, addr: SocketAddr) {
        if self.tcp_peers.lock().await.remove(&try_into_v4(addr)).is_some() {
            log::debug!("event=peer_tcp_closed from={}", addr);
        }
    }
}
