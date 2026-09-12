# Relay ticket validation over HTTP

Status: approved (2026-09-13; TLS requirement added). Owner: openuu-e9.

## Why

`hbbr` validates relay tickets by opening the account database in-process
(`hbbs::account::redeem_ticket` → `Accounts::redeem`, a `DELETE … RETURNING`
on `relay_tickets` in `OPENUU_ACCOUNT_DB`). That ties `hbbr` to the host that
owns the SQLite file, so the relay cannot move to a machine with more
bandwidth (docs/bandwidth.md, option 4c). Tickets are issued by
`openuu-account` (`POST /api/relay-ticket`, bearer session token) and carried
in `RequestRelay.token` to both `hbbs` and `hbbr`; that part does not change.

## Interface

`openuu-account` gains one internal endpoint:

```
POST /api/internal/relay-ticket/redeem
X-OpenUU-Internal: <secret>
{"ticket": "<64 hex>", "relay_id": "<uuid from RequestRelay>"}

200 {"ok": true}
200 {"ok": false, "reason": "unknown|expired|relay_mismatch|malformed"}
401 when the secret is missing or wrong
```

* Semantics are exactly `Accounts::redeem`: single use, bound to the relay
  id, expired rows purged on the way. A ticket is consumed by the first
  successful call, so there is nothing to cache on the relay side.
* The secret comes from `OPENUU_INTERNAL_SECRET` (env, or `_FILE` variant
  read at start-up) and is compared in constant time. When the variable is
  unset the route is not registered at all, so existing deployments are
  unaffected.
* The route is served on the normal account listener; the operator keeps it
  off the public interface with the firewall (only the relay host's address
  may reach 21114 for this path). A separate `OPENUU_INTERNAL_BIND` is not
  needed for one relay host and can be added later without changing `hbbr`.

`hbbr` gains an HTTP redeem path:

* `OPENUU_ACCOUNT_URL` (for example `http://10.0.0.5:21114`) plus
  `OPENUU_INTERNAL_SECRET` select it. Both unset → the current in-process
  SQLite path is used unchanged, so the same binary serves both layouts.
* One request per `RequestRelay` (each end of a relay redeems its own
  ticket, as today). Connect timeout 2 s, total timeout 3 s, one retry on a
  connection error only (never on a 200 with `ok:false`, and never on a
  timeout, because the first attempt may already have consumed the ticket).
* Fail closed: timeout, 5xx, malformed body or 401 all deny the relay and
  log `event=relay_ticket_http_error`. The client falls back to its own
  retry/relay-server rotation, which is the existing behaviour for a rejected
  ticket. No fail-open mode; a relay that cannot reach the account service is
  a misconfiguration to fix, not a state to run in.
* The HTTP client is `reqwest` (already a dependency of the workspace),
  built once at start-up with the timeouts above, no proxy.
* The relay host will be a different cloud machine, so the request crosses
  the public internet. `hbbr` therefore refuses to start unless
  `OPENUU_ACCOUNT_URL` is `https://…`, or `http://` to a loopback or private
  address (`127.0.0.0/8`, `10.0.0.0/8`, `172.16.0.0/12`, `192.168.0.0/16`,
  `::1`, `fc00::/7`). A literal host name is not resolved for this check;
  only `https` is accepted for names. In production a reverse proxy (caddy
  or nginx) terminates TLS in front of `openuu-account`, so the secret never
  travels in clear text. (Alternative kept as a note, not implemented: sign
  each request with `HMAC-SHA256(secret, timestamp + ticket + relay_id)` and
  reject a timestamp skew over 60 s; it needs replay protection on top and
  was judged not worth it while TLS termination is available.)

## Transition

1. Land the endpoint and the `hbbr` client behind the two variables. Deploy
   `openuu-account`/`hbbs` first; `hbbr` keeps using SQLite until told
   otherwise.
2. On the relay host set `OPENUU_ACCOUNT_URL` + `OPENUU_INTERNAL_SECRET` and
   drop `OPENUU_ACCOUNT_DB` from its unit; restart `hbbr`. Tickets already
   issued keep working because the table and its semantics are the same.
3. Once the relay has moved, remove the SQLite path from `hbbr` (separate
   change) so the binary no longer links the account store.

## Tests

* Account: handler test with the axum test client — valid ticket redeems
  once and is refused the second time; wrong relay id, expired, malformed
  and bad secret each return the documented answer.
* `hbbr`: the redeem function against a tiny in-process HTTP server —
  `ok:true` accepts, `ok:false` denies, a server that sleeps past the
  timeout denies, a refused connection denies after one retry.
* Both verified on the build machine with `cargo test`; nothing is deployed
  as part of this work.

## Out of scope

`hbbs` keeps reading the account database in-process (it stays on the
account host); moving its `authorized(token)` check to HTTP is a follow-up.
