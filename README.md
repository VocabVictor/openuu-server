# OpenUU Server

Self-hosted rendezvous and relay services for [OpenUU](https://github.com/VocabVictor/openuu).

## Programs

- `hbbs`: device ID registration, signaling and connection coordination.
- `hbbr`: relay traffic when direct connections are unavailable.
- `openuu-utils`: server diagnostics and key utilities.

## Build

```sh
git submodule update --init --recursive
cargo build --release
```

The binaries are written to `target/release`. Deployment examples are in `systemd/`, `docker-compose.yml` and `kubernetes/`.
Docker examples refer to locally built OpenUU images; no published image is assumed.
For a classic image, copy statically linked Linux `hbbs` and `hbbr` binaries to
`docker-classic/`, then run `docker build -t openuu-server:latest docker-classic`.
The Docker host must have this image before using the Compose example.
Service names and data directories now use `openuu`; migrate existing data and keys before switching an old deployment.
Run `hbbs --help` and `hbbr --help` for command-line options.

[Configuration and environment variables](docs/environment-variables.md)

## License and attribution

See [LICENSE](LICENSE) and [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md).
