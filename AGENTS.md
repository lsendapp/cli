# lsend CLI — Agent Integration Guide

Machine-readable, non-interactive CLI for the LocalSend protocol.

**Progressive docs (offline):** `lsend agent [scan|send|receive|errors|eval]`

**Cursor skill:** `skills/localsend-cli/SKILL.md` (install to `.cursor/skills/` or use `npx skills add` if published)

## Principles

- No prompts — all inputs via flags
- **`--json`** — structured stdout only (no human text); also enabled when stdout is piped or `LOCALSEND_NO_TUI=1`
- **`--quiet`** — minimal stdout in human mode (`--json` implies quiet)
- Logs on **stderr** via `-v` / `RUST_LOG`
- Stable **exit codes** + **`code`** field in error JSON

## Global flags

| Flag | Description |
|------|-------------|
| `--json` | JSON stdout (`scan`/`send` object, `receive` NDJSON); auto-enabled when piped |
| `--quiet` / `-q` | Suppress progress text (human mode) |
| `LOCALSEND_NO_TUI=1` | Same as `--json` for non-interactive stdout |
| `-v, --verbose` | Diagnostics on stderr |
| `--http` | Plain HTTP (default: HTTPS) |
| `--port PORT` | Default `53317` |
| `--alias NAME` | Local display name |

## Exit codes

| Code | Meaning |
|------|---------|
| 0 | Success |
| 1 | General error (`code: "error"`) |
| 2 | Not found (`target_not_found`, `no_files`) |
| 3 | Port in use (`port_in_use`) |

## scan

```bash
lsend scan --json --timeout 5000
```

```json
{
  "command": "scan",
  "ok": true,
  "timeout_ms": 5000,
  "devices": [{ "alias": "...", "ip": "192.168.1.10", "port": 53317, "https": true, ... }]
}
```

Empty `devices` with `ok: true` is not an error.

## send

**Always prefer IP** from scan. Use **`--no-scan`** to forbid slow alias lookup.

```bash
lsend scan --json --timeout 5000
lsend send 192.168.1.10 ./file.pdf --json --no-scan
```

```json
{
  "command": "send",
  "ok": true,
  "target": { "ip": "192.168.1.10", ... },
  "resolved_via": "ip",
  "files": [{ "name": "file.pdf", "path": "/abs/file.pdf", "size": 1024, "status": "sent" }]
}
```

`resolved_via`: `"ip"` (preferred) or `"scan"` (alias triggered discovery).

## receive

```bash
lsend receive --json --once --dir /tmp/inbox
lsend receive --json --once --pin 123456 --dir /tmp/inbox
```

Receive PIN priority: `--pin` (persisted to `receive_pin`) > config file > `LSEND_RECEIVE_PIN` env.

NDJSON events: `ready` → `transfer_started` → `file_saved` → `transfer_complete` → `shutdown`

## Errors (`--json`)

```json
{
  "ok": false,
  "command": "send",
  "code": "target_not_found",
  "error": "No device found with alias \"...\". Run `lsend scan --json` first or pass an IP address.",
  "hint": "Run `lsend scan --json` first and use the device IP, or pass an IP address directly."
}
```

## Agent eval checklist

See `lsend agent eval` for a step-by-step smoke test.

## Notes

- Do not run `receive` while the official app holds port 53317
- Identity (TLS) stored in `~/.config/lsend/`
- Device alias persisted in `alias.txt` (official LocalSend word lists + system locale; `--alias` overrides for one run)
- Receive PIN via `receive --pin` (persisted in `receive_pin`) or `LSEND_RECEIVE_PIN`
