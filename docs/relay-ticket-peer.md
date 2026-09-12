# Relay tickets for an unattended controlled peer

Status: approved and implemented 2026-09-13 (server eb22859 + b3bb231, client
e5418fda2); not yet deployed. Independent of docs/relay-ticket-http.md.

## Problem

`hbbr` pairs a relay only when *both* ends present a valid ticket in
`RequestRelay.token`. The controlled peer obtains its ticket in
`src/server/connect.rs` (`create_relay_connection_`) by calling
`account::require_login()` and `POST /api/relay-ticket` with its own session
token. An unattended machine has no session, so its `RequestRelay` is never
sent and the relay times out after 30 s (observed 2026-09-13 on the CVM:
one ticket issued per uuid, `hbbr` sees one side only).

Ruling: login and tickets constrain the controller only; the controlled
peer needs no account.

## Design: hbbs issues the peer's ticket and forwards it

1. **Controller** (unchanged): sends `RequestRelay{id, uuid, token: <session
   token>, relay_server}` to `hbbs` over `secure_tcp`, then fetches its own
   ticket and connects to `hbbr`.
2. **hbbs** (`src/rendezvous_server/tcp.rs`, `RequestRelay` arm): today it
   checks `authorized(rf.token)`, clears the token and forwards the message
   to the peer. Change: instead of clearing, replace `rf.token` with a
   *peer ticket* minted by the account store for this uuid:
   `Accounts::peer_ticket(session_token, uuid)` inserts a `relay_tickets`
   row exactly like `Accounts::ticket` (same 60 s expiry, same binding to
   the controller's session hash and to `relay_id = uuid`). `hbbs` already
   opens the account database in-process for `authorized`, so no new
   dependency. Log `event=relay_peer_ticket uuid=…`.
3. **Controlled peer** (`create_relay_connection_`): if the forwarded
   `RequestRelay.token` is non-empty, use it as the ticket and skip
   `require_login()` / `relay_ticket()`. If it is empty (old `hbbs`), fall
   back to today's path. The `token` field already exists in the proto; no
   proto change and no new message.
4. **hbbr / account store / relay-ticket HTTP endpoint**: unchanged. Each
   ticket is still single-use and bound to one uuid; the two ends simply
   hold two different rows.

### Why two single-use tickets rather than one ticket with two uses

* No change to `Accounts::redeem`, `hbbr` or the just-built HTTP endpoint;
  the "consumed by the first successful call" property that the HTTP
  design relies on (no retry after a 200) stays exactly true.
* A ticket that could be redeemed twice would let whichever side holds it
  connect twice; with two rows the controller's ticket still admits one
  stream and the peer's ticket one stream, so a replay by either side is
  refused by the store rather than by luck of ordering.
* Revocation and the audit trail stay per row: both rows carry the
  controller's `session_hash`, so `revoke`/`disable` still deletes them.

### Security notes

* The peer ticket travels `hbbs -> peer` on the peer's registration channel
  (UDP in the common case, not encrypted). An eavesdropper on that path
  could redeem it first and be paired with the controller's relay stream.
  That is the same position any relay operator is in today: the session
  itself is end-to-end encrypted with the peer's key pair, so the attacker
  gets a stream it cannot read and the legitimate peer's attempt is refused
  and logged (`event=relay_denied`). The controller's own ticket already
  crosses the same relay. Accepted; no new exposure of the account
  session (the peer never sees a session token).
* The peer ticket is bound to the uuid `hbbs` received from the controller,
  so it cannot be used for a different relay.
* `hbbs` mints a ticket only after `authorized(rf.token)` passed, so an
  unauthenticated `RequestRelay` still produces nothing.

### Compatibility

* Old peer + new hbbs: the peer ignores the forwarded token and asks for its
  own ticket as today (works only if it is logged in, as today).
* New peer + old hbbs: token arrives empty, the peer falls back to today's
  path.
* Old controller: unaffected; it never sees the peer ticket.
* When every peer is updated, `require_login()` and `relay_ticket()` can be
  removed from `create_relay_connection_` in a follow-up.

## Known limitations

* The peer ticket travels `hbbs -> peer` on the peer's registration channel,
  which is plain UDP for most peers. An eavesdropper on that path can
  redeem the ticket first and take the peer's place on the relay; the
  session stays end-to-end encrypted, so this is a denial of that one
  attempt, logged as `event=relay_denied`. Follow-up: encrypt the peer's
  registration channel (the UDP path has no key exchange today).
* Only relays requested by the controller through `hbbs`
  (`handle_request_relay`) carry a forwarded ticket. When the *peer*
  decides to relay on its own after a punch-hole or intranet attempt
  (`rendezvous_mediator/punch.rs`, `relay.rs::handle_intranet` with
  `initiate = true`) it generates the uuid itself, `hbbs` never sees it and
  the peer still needs its own login for that attempt. Covering it needs
  either `hbbs` to answer the peer's `RelayResponse` with a ticket (state
  per controller connection, a reply on the peer's socket) or the
  controller to route those relays through `hbbs` as well; both are a
  separate design.

## Changes and tests

* `openuu-server`: `Accounts::peer_ticket` (+ store test: row bound to uuid
  and session, redeemable once by `hbbr`), `hbbs` forwarding (+ unit test
  on the `RequestRelay` arm through `handle_tcp` with the in-memory server
  from `secure_tests.rs`: forwarded message carries a 64-hex token that
  `redeem` accepts for that uuid and refuses for another). Two commits.
* `openuu`: `create_relay_connection_` prefers the forwarded token (one
  commit); `relay_token` is unit-tested for both branches (forwarded ticket
  used as is, empty ticket falls back to the login check) and the whole
  path by the CVM smoke test with the temporary login removed from the
  controlled peers.
* Order: server first (deploy), then client; the temporary session tokens
  on the two test peers are removed after the client build is installed.
