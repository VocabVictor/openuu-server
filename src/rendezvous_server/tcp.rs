use super::*;

impl RendezvousServer {
    #[inline]
    pub(super) async fn handle_tcp(
        &mut self,
        bytes: &[u8],
        sink: &mut Option<Sink>,
        addr: SocketAddr,
        key: &str,
        ws: bool,
    ) -> bool {
        if let Ok(msg_in) = RendezvousMessage::parse_from_bytes(bytes) {
            match msg_in.union {
                Some(rendezvous_message::Union::PunchHoleRequest(ph)) => {
                    if !crate::account::authorized(&ph.token).await {
                        log::warn!("event=punch_unauthorized from={} peer={}", addr, ph.id);
                        return false;
                    }
                    // there maybe several attempt, so sink can be none
                    if let Some(sink) = sink.take() {
                        self.tcp_punch.lock().await.insert(try_into_v4(addr), sink);
                    }
                    self.punch_sessions.remember(addr, &ph.id, &ph.token).await;
                    allow_err!(self.handle_tcp_punch_hole_request(addr, ph, key, ws).await);
                    return true;
                }
                Some(rendezvous_message::Union::RequestRelay(mut rf)) => {
                    if !crate::account::authorized(&rf.token).await {
                        log::warn!("event=relay_request_unauthorized from={} peer={}", addr, rf.id);
                        return false;
                    }
                    // An unattended peer has no session of its own: hand it a ticket bound to
                    // this uuid (docs/relay-ticket-peer.md). Old peers ignore it and, like a
                    // peer that gets an empty token from an old hbbs, fetch their own.
                    rf.token = crate::account::peer_ticket(&rf.token, &rf.uuid)
                        .await
                        .unwrap_or_default();
                    // there maybe several attempt, so sink can be none
                    if let Some(sink) = sink.take() {
                        self.tcp_punch.lock().await.insert(try_into_v4(addr), sink);
                    }
                    if let Some(peer) = self.pm.get_in_memory(&rf.id).await {
                        let peer_addr = peer.read().await.socket_addr;
                        log::info!(
                            "event=relay_request from={} id={} peer={} relay={} uuid={}",
                            addr,
                            rf.id,
                            peer_addr,
                            rf.relay_server,
                            rf.uuid
                        );
                        let mut msg_out = RendezvousMessage::new();
                        rf.socket_addr = AddrMangle::encode(addr).into();
                        msg_out.set_request_relay(rf);
                        self.tx.send(Data::Msg(msg_out.into(), peer_addr)).ok();
                    } else {
                        log::info!(
                            "event=relay_request from={} id={} decision=peer_not_in_memory",
                            addr,
                            rf.id
                        );
                    }
                    return true;
                }
                Some(rendezvous_message::Union::RelayResponse(mut rr)) => {
                    let addr_b = AddrMangle::decode(&rr.socket_addr);
                    self.answer_peer_initiated_relay(&rr, addr_b, sink, addr).await;
                    rr.socket_addr = Default::default();
                    let id = rr.id();
                    if !id.is_empty() {
                        let pk = self.get_pk(&rr.version, id.to_owned()).await;
                        rr.set_pk(pk);
                    }
                    let mut msg_out = RendezvousMessage::new();
                    if !rr.relay_server.is_empty() {
                        if self.is_lan(addr_b) {
                            // https://github.com/rustdesk/rustdesk-server/issues/24
                            rr.relay_server = self.inner.local_ip.clone();
                        } else if rr.relay_server == self.inner.local_ip {
                            rr.relay_server = self.get_relay_server(addr.ip(), addr_b.ip());
                        }
                    }
                    log::info!(
                        "event=relay_response from={} to={} relay={} refuse={}",
                        addr,
                        addr_b,
                        rr.relay_server,
                        rr.refuse_reason
                    );
                    msg_out.set_relay_response(rr);
                    allow_err!(self.send_to_tcp_sync(msg_out, addr_b).await);
                }
                Some(rendezvous_message::Union::PunchHoleSent(phs)) => {
                    allow_err!(self.handle_hole_sent(phs, addr, None).await);
                }
                Some(rendezvous_message::Union::LocalAddr(la)) => {
                    allow_err!(self.handle_local_addr(la, addr, None).await);
                }
                Some(rendezvous_message::Union::TestNatRequest(tar)) => {
                    let mut msg_out = RendezvousMessage::new();
                    let mut res = TestNatResponse {
                        port: addr.port() as _,
                        ..Default::default()
                    };
                    if self.inner.serial > tar.serial {
                        let mut cu = ConfigUpdate::new();
                        cu.serial = self.inner.serial;
                        cu.rendezvous_servers = (*self.rendezvous_servers).clone();
                        res.cu = MessageField::from_option(Some(cu));
                    }
                    msg_out.set_test_nat_response(res);
                    self.reply_on_connection(sink, addr, msg_out).await;
                }
                Some(rendezvous_message::Union::RegisterPeer(rp)) => {
                    self.handle_tcp_register_peer(rp, sink, addr).await;
                    // keep the registration connection open (a false return closes it)
                    return true;
                }
                Some(rendezvous_message::Union::RegisterPk(rk)) => {
                    self.handle_tcp_register_pk(rk, sink, addr).await;
                    return true;
                }
                _ => {}
            }
        }
        false
    }
    #[inline]
    pub(super) async fn send_to_tcp(&mut self, msg: RendezvousMessage, addr: SocketAddr) {
        let mut tcp = self.tcp_punch.lock().await.remove(&try_into_v4(addr));
        if tcp.is_none() {
            // the request came on a connection that registered the peer (tcp_register.rs)
            self.send_to_tcp_peer(addr, msg).await;
            return;
        }
        tokio::spawn(async move {
            Self::send_to_sink(&mut tcp, msg).await;
        });
    }

    #[inline]
    pub(super) async fn send_to_sink(sink: &mut Option<Sink>, msg: RendezvousMessage) {
        if let Some(sink) = sink.as_mut() {
            if let Ok(bytes) = msg.write_to_bytes() {
                match sink {
                    Sink::TcpStream(s, enc) => {
                        let bytes = match enc.as_mut() {
                            Some(enc) => enc.enc(&bytes),
                            None => bytes,
                        };
                        allow_err!(s.send(Bytes::from(bytes)).await);
                    }
                    Sink::Ws(ws) => {
                        allow_err!(ws.send(tungstenite::Message::Binary(bytes)).await);
                    }
                    Sink::Closed => {}
                }
            }
        }
    }

    #[inline]
    pub(super) async fn send_to_tcp_sync(
        &mut self,
        msg: RendezvousMessage,
        addr: SocketAddr,
    ) -> ResultType<()> {
        let mut sink = self.tcp_punch.lock().await.remove(&try_into_v4(addr));
        if sink.is_none() {
            // the request came on a connection that registered the peer (tcp_register.rs)
            self.send_to_tcp_peer(addr, msg).await;
            return Ok(());
        }
        Self::send_to_sink(&mut sink, msg).await;
        Ok(())
    }
    #[inline]
    pub(super) async fn get_pk(&mut self, version: &str, id: String) -> Bytes {
        if version.is_empty() || self.inner.sk.is_none() {
            Bytes::new()
        } else {
            match self.pm.get(&id).await {
                Some(peer) => {
                    let pk = peer.read().await.pk.clone();
                    sign::sign(
                        &hbb_common::message_proto::IdPk {
                            id,
                            pk,
                            ..Default::default()
                        }
                        .write_to_bytes()
                        .unwrap_or_default(),
                        self.inner.sk.as_ref().unwrap(),
                    )
                    .into()
                }
                _ => Bytes::new(),
            }
        }
    }

    #[inline]
    pub(super) fn get_server_sk(key: &str) -> (String, Option<sign::SecretKey>) {
        let mut out_sk = None;
        let mut key = key.to_owned();
        if let Ok(sk) = base64::decode(&key) {
            if sk.len() == sign::SECRETKEYBYTES {
                log::info!("The key is a crypto private key");
                key = base64::encode(&sk[(sign::SECRETKEYBYTES / 2)..]);
                let mut tmp = [0u8; sign::SECRETKEYBYTES];
                tmp[..].copy_from_slice(&sk);
                out_sk = Some(sign::SecretKey(tmp));
            }
        }

        if key.is_empty() || key == "-" || key == "_" {
            let (pk, sk) = crate::common::gen_sk(0);
            out_sk = sk;
            if !key.is_empty() {
                key = pk;
            }
        }

        if !key.is_empty() {
            log::info!("Key: {}", key);
        }
        (key, out_sk)
    }

    #[inline]
    pub(super) fn is_lan(&self, addr: SocketAddr) -> bool {
        if let Some(network) = &self.inner.mask {
            match addr {
                SocketAddr::V4(v4_socket_addr) => {
                    return network.contains(*v4_socket_addr.ip());
                }

                SocketAddr::V6(v6_socket_addr) => {
                    if let Some(v4_addr) = v6_socket_addr.ip().to_ipv4() {
                        return network.contains(v4_addr);
                    }
                }
            }
        }
        false
    }
}

impl RendezvousServer {
    /// A RelayResponse carrying a uuid means the controlled peer chose to relay on its own.
    /// Hand it a peer ticket minted from the controller's session seen in the punch request,
    /// on the socket it just used (docs/relay-ticket-peer-initiated.md). Nothing is sent when
    /// there is no matching session; the peer then falls back to its own login.
    async fn answer_peer_initiated_relay(
        &self,
        rr: &RelayResponse,
        controller: SocketAddr,
        sink: &mut Option<Sink>,
        from: SocketAddr,
    ) {
        if rr.uuid.is_empty() {
            return;
        }
        let Some(token) = self.punch_sessions.token_for(controller, rr.id()).await else {
            return;
        };
        let Some(ticket) = crate::account::peer_ticket(&token, &rr.uuid).await else {
            return;
        };
        let mut back = RendezvousMessage::new();
        back.set_request_relay(RequestRelay {
            uuid: rr.uuid.clone(),
            token: ticket,
            ..Default::default()
        });
        self.reply_on_connection(sink, from, back).await;
        log::info!("event=relay_peer_ticket uuid={} via=response", rr.uuid);
    }
}
