# Tencent deployment

Verified 2026-09-13 on CVM ins-xxxxxxxx, ap-nanjing (rs.example.com).

- Account API: http://rs.example.com:21114
- ID server: rs.example.com:21116
- Relay server: rs.example.com:21117
- Services: openuu-account, openuu-hbbs, openuu-hbbr (systemd, enabled at boot).
- Runtime user: openuu. Binaries: /opt/openuu-server/bin.
- State: /var/lib/openuu-server, shared OPENUU_ACCOUNT_DB=openuu-accounts.sqlite3 in that directory.
- Account listener: OPENUU_ACCOUNT_BIND=0.0.0.0:21114.
- hbbs arguments: -r rs.example.com:21117 -k _. hbbr arguments: -k _.
- Security group sg-0123456789 allows TCP 21114-21119 and UDP 21116.
- Public key: /var/lib/openuu-server/id_ed25519.pub. Private key stays on the server.
- Test accounts: test-account-1, test-account-2. Passwords are kept outside Git in the operator's restricted local credential file.

Built Linux x86_64 release with Rust 1.92.0, one job, vendored dependencies and SODIUM_USE_PKG_CONFIG=1. A separate empty SQLite peer schema was used for SQLx compile-time checking via DATABASE_URL; it is not the runtime account database.

Verified from the operator's Windows PC over the public IP: both account logins, current-user validation, wrong-password rejection, unauthenticated API rejection, authenticated relay bidirectional payload, anonymous relay rejection, logout token revocation. Installed binaries match build outputs. All three services are active and enabled.

## 2026-09-13 update (commits ad8b9af, 08434f4, 49fbd73)

Rebuilt on the CVM in the existing /home/<user>/openuu-build tree (vendored dependencies unchanged, Cargo.lock unchanged): `cargo --config vendor.toml build --offline --locked --release --bins -j 1` with `DATABASE_URL=sqlite://compile-schema.sqlite3` and `SODIUM_USE_PKG_CONFIG=1`, Rust 1.92.0; incremental release build took 2 min 57 s. Previous binaries are kept in /opt/openuu-server/bin.bak-20260913 for rollback.

Installed SHA-256:

- openuu-account `530079031e2d2e3702e429dfda8b599d2b6be23ffcd1c1a08c558d85fc575528`
- hbbs `4585b98d1fbc76cd379328db0f4ff6d20a0ce63ca5ca5859d40d2e5fddb5107a`
- hbbr `2da79a1ba1c3de432c9960365a69f091de0f085c23ea3bda7b927cc30a2e5124`

Unit files unchanged (`openuu-account` without arguments still runs the HTTP service). All three services restarted, active and enabled. Verified: `/opt/openuu-server/healthcheck.sh` HEALTHY, `verify-deployment.py` passed (both logins, wrong password, unauthenticated API, authenticated relay round trip, anonymous relay rejection, logout revocation), `openuu-account list` shows test-account-1 and test-account-2 enabled, journal now carries `event=login_ok|login_failed|relay_ticket_issued|relay_denied` lines and no sqlx statement output. Operations guide: [operations.md](operations.md).

The current CVM outgoing bandwidth setting is 50 Mbps. This limits relay throughput. The native client must be rebuilt with the account/ticket changes to use authenticated relay; a Flutter-only build with the old native DLL is insufficient.
