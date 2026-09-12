# OpenUU account service

`openuu-account` provides account/password login. Public registration and third-party OAuth are not enabled. Accounts are created by the server operator.

## Build and start

Build all programs with `cargo build --release`. Run `openuu-account`, `hbbs` and `hbbr` with the same absolute `OPENUU_ACCOUNT_DB` path. The default is `openuu-accounts.sqlite3` in the working directory. Restrict filesystem access to this database and its directory.

`openuu-account` listens on `127.0.0.1:21114` by default. The client accepts HTTP and HTTPS account URLs. HTTPS is recommended; an explicitly configured HTTP URL sends account credentials and session tokens without transport encryption. `OPENUU_ACCOUNT_BIND` overrides the bind address. The service does not trust forwarded IP headers; login rate limits apply to its direct TCP peer.

Create a user by setting `OPENUU_NEW_PASSWORD` in the process environment, then running `openuu-account create USERNAME`. The process creates the account and exits. Names accept ASCII letters, digits, dots, underscores and hyphens (1-64 characters); passwords must be 12-72 UTF-8 bytes. Clear the environment variable after use. There is no default password. The old `openuu-account USERNAME` form is no longer accepted.

The same binary manages existing accounts: `passwd NAME` (new password from `OPENUU_NEW_PASSWORD`, revokes sessions), `disable NAME` / `enable NAME`, `delete NAME` and `list`. See [operations.md](operations.md) for usage on the deployed server.

Configure the HTTP or HTTPS base URL in the client's Network / API server setting. Configure the matching ID/relay server and public key separately. Both controlling and controlled OpenUU devices must sign in. An account login does not replace the controlled device's password or approval requirements.

## Authentication behavior

- Passwords are bcrypt hashes; sessions are random opaque tokens stored only as SHA-256 hashes on the server.
- Sessions expire after 24 hours. Logout revokes the session immediately at the API. Account disablement (`accounts.enabled=0`) invalidates its sessions.
- The native client checks the account service before connecting and periodically during active desktop/file/tunnel sessions. Cached successful checks last at most five seconds; HTTP checks have a five-second timeout. Failures deny access.
- `hbbs` rejects unauthenticated connection coordination. `hbbr` requires a short-lived, single-use ticket scoped to the relay UUID. Account session tokens are never sent to the relay handshake.
- All three services must share the account database. There is no anonymous fallback. Old clients that do not request relay tickets cannot use this relay.
- The caller's account credentials are never forwarded to the controlled device.

## API

| Method | Path | Result |
| --- | --- | --- |
| POST | `/api/login` | Existing client `access_token` response and user profile |
| POST/GET | `/api/currentUser` | Validates bearer session and returns user profile |
| POST | `/api/logout` | Revokes bearer session |
| GET | `/api/login-options` | Empty list (password login only) |
| POST | `/api/relay-ticket` | Authenticated one-use ticket for supplied `uuid` |

Account-scoped cloud address book and device groups are not implemented; their initial list endpoints return empty results, and writes are not accepted. Remote connections still use device IDs.

## Verification

`cargo test --lib account::` covers password failures, invalid/expired/revoked sessions, relay-ticket scope and single use, HTTP compatibility, login rate limiting and the account administration commands. `cargo check --bins` covers all service binaries.

A full native OpenUU client build is required for connection enforcement. A Flutter-only build with an old precompiled native library does **not** enforce this policy on native paths.
