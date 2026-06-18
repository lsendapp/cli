# lsend

Headless command-line tool for sending and receiving files over the local network, compatible with the [LocalSend](https://localsend.org) protocol.

## Requirements

- Rust 1.75+

> The LocalSend Rust `core` crate is vendored under `vendor/localsend-core/` — no need to clone the upstream repository.

## Build

```bash
git clone <this-repo-url>
cd lsend
cargo build --release
# binary at: target/release/lsend
```

To install into `~/.cargo/bin/`:

```bash
cargo install --path .
```

## Usage

```bash
# Discover nearby devices
lsend scan

# Receive files (auto-accept, saves to ~/Downloads by default)
lsend receive

# Send files to a device (IP address or alias from scan)
lsend send 192.168.1.42 ./photo.png ./notes.txt
```

### Global options

| Flag | Description |
|------|-------------|
| `--http` | Use plain HTTP instead of HTTPS |
| `--port PORT` | Listen/connect port (default: 53317) |
| `--alias NAME` | Device display name |

### Per-subcommand options

| Subcommand | Flag | Description |
|------------|------|-------------|
| `scan` | `--timeout MS` | How long to wait for responses, in milliseconds (default: 500) |
| `send` | `--pin PIN` | PIN when sending to a PIN-protected receiver |
| `receive` | `--dir PATH` | Directory where received files are saved (default: Downloads) |

## Configuration

Identity (TLS certificate and fingerprint) is stored under:

- Linux/macOS: `~/.config/lsend/`
- Windows: `%APPDATA%\lsend\`

## License

`lsend-cli` is licensed under the [Apache License 2.0](LICENSE).

This project vendors the `core` crate from the [LocalSend](https://github.com/localsend/localsend) project,
which is also licensed under the [Apache License 2.0](vendor/localsend-core/LICENSE) — Copyright 2022-2024 Tien Do Nam.
