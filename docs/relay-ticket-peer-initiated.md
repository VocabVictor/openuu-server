# Peer tickets for relays the controlled peer initiates

Status: proposal (openuu-e9, 2026-09-13), awaiting lenovo-0f. Extends
docs/relay-ticket-peer.md, which covers only relays the controller requests
through `hbbs`.

## Gap

After a punch-hole or intranet attempt the controlled peer may decide on its
own to relay (`rendezvous_mediator/punch.rs` on symmetric NAT or forced
relay, `relay.rs::handle_intranet` when the direct attempt fails). It then:

1. generates `uuid`, opens a TCP connection to `hbbs` and sends
   `RelayResponse{uuid, relay_server, id: <its own id>, socket_addr:
   <controller addr from the PunchHole/FetchLocalAddr it received>}`;
2. immediately calls `create_relay_connection` and connects to `hbbr` with a
   ticket it must fetch with its own login.

`hbbs` forwards the `RelayResponse` to the controller, which connects to
`hbbr` directly with a ticket of its own (`client/secure.rs::create_relay`);
it never sends a `RequestRelay` for this uuid, so no forwarded peer ticket
exists and an unattended peer fails at step 2.

## Options

**a (0f's variant, recommended): `hbbs` remembers the controller's session
and answers the peer's `RelayResponse` with a ticket.**

* `hbbs`, `PunchHoleRequest` arm (`tcp.rs`): after `authorized(ph.token)`,
  store `(controller addr as v4, ph.id) -> (ph.token, now)` in a new
  `Mutex<HashMap>` on `RendezvousServer` beside `tcp_punch`; entries older
  than 60 s are dropped on insert (a punch-hole that has not resolved in
  60 s is dead anyway). Same for the `FetchLocalAddr`-producing path,
  which starts from the same request.
* `hbbs`, `RelayResponse` arm: if `rr.uuid` is non-empty, look up
  `(AddrMangle::decode(rr.socket_addr), rr.id())`. On a hit, mint
  `account::peer_ticket(token, rr.uuid)` and send
  `RequestRelay{uuid, token: <ticket>}` back on the *peer's own socket*
  (the `sink` `handle_tcp` already holds) before forwarding the
  `RelayResponse` to the controller as today. On a miss (old flow, expired,
  or a peer answering something `hbbs` never saw) send nothing; the peer
  falls back as below. Log `event=relay_peer_ticket uuid=… via=response`.
* Peer, `rendezvous_mediator/relay.rs::create_relay` with `initiate =
  true`: after `socket.send(RelayResponse)`, and **only when this peer has
  no session token of its own** (`account::session_token().is_empty()`),
  wait up to 1.5 s (`socket.next_timeout`) for a `RequestRelay` whose
  `uuid` matches and pass its `token` to `create_relay_connection`. A peer
  that is logged in keeps today's timing and path exactly. If nothing
  arrives the empty ticket falls through to `relay_token`, which fails with
  "Sign in" as today.
* Controller: unchanged. Proto: unchanged (`RequestRelay` already has
  `uuid` and `token`).

Compatibility: old `hbbs` + new peer: the unattended peer waits 1.5 s, then
fails as it does today; a logged-in peer is unaffected. New `hbbs` + old
peer: the reply is written to a socket the old peer never reads and closes
right after sending; harmless. Nothing depends on the controller version.

Cost: one map on `hbbs` (entries live 60 s, keyed by a controller address
that must have passed `authorized`), one extra message on a socket that is
already open, no new round trip for the controller.

**b: the controller routes these relays through `hbbs` too.**

On `RelayResponse{uuid}` the controller would send `RequestRelay{id, uuid,
token}` to `hbbs` instead of connecting to `hbbr` directly, so the existing
forwarding path delivers a peer ticket. But the peer has *already* connected
to `hbbr` in step 2, so it would also have to stop doing that and wait for
the forwarded `RequestRelay`. That changes both ends at once and has no
compatible mixed state: old controller + new peer never pairs (the peer
waits for a request that never comes); new controller + old peer connects
twice. It also adds a controller-to-`hbbs` round trip on the slow path and
touches `client/start_inner.rs` (an exception file). Rejected.

## Security

* The ticket is minted only from a session that passed `authorized` within
  the last 60 s and only for the `(controller, peer id)` pair that request
  named; a peer cannot obtain a ticket for a controller that never asked
  for it.
* The reply goes to the peer over the TCP socket the peer opened to `hbbs`
  (plain TCP; the peer's registration socket does not run `secure_tcp`).
  Same exposure as the UDP delivery already accepted in
  docs/relay-ticket-peer.md; the follow-up "encrypt the peer's channel"
  covers both.
* The map is keyed by address; a controller behind a NAT that changes its
  source port between the punch request and the relay is a miss, not a
  wrong hit, and the peer falls back.

## Changes and tests

* `openuu-server`, two commits: the session map + `PunchHoleRequest` hook
  (test: entry present, expired entry dropped); the `RelayResponse` reply
  (test through `handle_tcp` with the in-memory server: after a punch
  request from A for id P, a `RelayResponse` from P naming A gets a
  `RequestRelay` back on P's sink with a 64-hex token redeemable once for
  that uuid; without a prior punch request nothing is sent).
* `openuu`, one commit in `rendezvous_mediator/relay.rs::create_relay`
  (wait-for-ticket branch gated on an empty session token; unit test for
  the gate function, path covered by the CVM smoke test on a symmetric-NAT
  or forced-relay peer).
