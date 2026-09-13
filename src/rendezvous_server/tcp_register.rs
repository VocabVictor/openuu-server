use super::*;

/// Peer registration over the (secured) TCP rendezvous connection. The UDP path in
/// `udp.rs` is unchanged; both share `register_peer_response` / `register_pk_result`.
/// A peer registered here keeps its connection's sink in `tcp_peers`, keyed by the
/// connection address that is also stored as the peer's `socket_addr`, so every
/// message hbbs would push to that address over UDP (`PunchHole`, `RequestRelay`,
/// `FetchLocalAddr`, ...) is delivered through the encrypted TCP stream instead.
///
/// Registration moves the connection's sink into `tcp_peers`, so later requests on the
/// same connection see `sink == None` in `handle_tcp`; their replies go through
/// `reply_on_connection` / the `tcp_punch` fallback in `send_to_tcp*`, which look the
/// sink up here.
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

    /// Sends to a TCP-registered peer without giving up its sink. The sink is taken out
    /// of the map for the duration of the send so the lock is never held across the
    /// `.await`; it is put back afterwards unless the connection was forgotten (or the
    /// address re-registered) in the meantime. Returns false when no such peer is
    /// connected, so the caller can fall back to UDP.
    pub(super) async fn send_to_tcp_peer(&self, addr: SocketAddr, msg: RendezvousMessage) -> bool {
        let key = try_into_v4(addr);
        let taken = match self.tcp_peers.lock().await.get_mut(&key) {
            Some(sink) => std::mem::replace(sink, Sink::Closed),
            None => return false,
        };
        if matches!(taken, Sink::Closed) {
            // another send is in flight on this connection; the message is dropped like a
            // lost UDP datagram and the peer's next heartbeat/retry recovers
            return true;
        }
        let mut slot = Some(taken);
        Self::send_to_sink(&mut slot, msg).await;
        if let Some(s) = slot {
            let mut peers = self.tcp_peers.lock().await;
            if let Some(entry) = peers.get_mut(&key) {
                if matches!(entry, Sink::Closed) {
                    *entry = s;
                }
            }
        }
        true
    }

    /// Answers on the connection a request arrived on: through `sink` while the
    /// connection still owns it, otherwise through `tcp_peers` (the sink moved there
    /// when the peer registered on this connection).
    pub(super) async fn reply_on_connection(&self, sink: &mut Option<Sink>, addr: SocketAddr, msg: RendezvousMessage) {
        if sink.is_some() {
            Self::send_to_sink(sink, msg).await;
        } else {
            self.send_to_tcp_peer(addr, msg).await;
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

    /// Periodic share of TCP-registered peers among online peers, for judging when the
    /// UDP registration path can be retired.
    pub(super) async fn log_peer_transport(&self) {
        let online = self.pm.online_count(REG_TIMEOUT).await;
        let tcp = self.tcp_peers.lock().await.len();
        log::info!(
            "event=peer_transport online={} tcp={} udp={}",
            online,
            tcp,
            online.saturating_sub(tcp)
        );
    }

    /// Forgets the TCP sink of a closed connection.
    pub(super) async fn forget_tcp_peer(&self, addr: SocketAddr) {
        if self.tcp_peers.lock().await.remove(&try_into_v4(addr)).is_some() {
            log::debug!("event=peer_tcp_closed from={}", addr);
        }
    }
}
