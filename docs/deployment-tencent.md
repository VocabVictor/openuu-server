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

The current CVM outgoing bandwidth setting is 50 Mbps. This limits relay throughput. The native client must be rebuilt with the account/ticket changes to use authenticated relay; a Flutter-only build with the old native DLL is insufficient.
