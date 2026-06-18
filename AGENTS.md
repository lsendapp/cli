# lsend CLI — Agent Integration Guide

Machine-readable, non-interactive CLI for the LocalSend protocol.

**Progressive docs (offline):** `lsend agent [alias|scan|send|receive|errors|eval]`

**Cursor skill:** `skills/localsend-cli/SKILL.md` (install to `.cursor/skills/` or use `npx skills add` if published)

## Principles

- No prompts — all inputs via flags
- **`--json`** — structured stdout only (no human text); also enabled when stdout is piped or `LSEND_NO_TUI=1`
- **`--quiet`** — minimal stdout in human mode (`--json` implies quiet)
- Logs on **stderr** via `-v` / `RUST_LOG`
- Stable **exit codes** + **`code`** field in error JSON

## Global flags

| Flag | Description |
|------|-------------|
| `--json` | JSON stdout (`scan`/`send` object, `receive` NDJSON); auto-enabled when piped |
| `--quiet` / `-q` | Suppress progress text (human mode) |
| `LSEND_NO_TUI=1` | Same as `--json` for non-interactive stdout |
| `-v, --verbose` | Diagnostics on stderr |
| `--http` | Plain HTTP (default: HTTPS) |
| `--port PORT` | Default `53317`. **Keep 53317 for receive** — alternate ports break multicast discovery (the LocalSend app and default `scan` will not see this device). |
| `--alias NAME` | Override persisted alias for one command only |

## Exit codes

| Code | Meaning |
|------|---------|
| 0 | Success |
| 1 | General error (`code: "error"`) |
| 2 | Not found / validation (`target_not_found`, `no_files`, `invalid_alias`) |
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

## alias

Manage the persisted device alias (the LocalSend app UI: **Device name**). Stored in `~/.config/lsend/alias.txt`.

```bash
lsend alias show --json
lsend alias regenerate --json
lsend alias regenerate --json --locale zh-CN
lsend alias set "My Laptop" --json
```

`lsend alias` without a subcommand is the same as `alias show`.

Regenerate JSON:

```json
{
  "command": "alias",
  "action": "regenerate",
  "ok": true,
  "previous": "Adorable Orange",
  "alias": "Clever Banana",
  "path": "/Users/me/.config/lsend/alias.txt",
  "locale": "en"
}
```

Priority for network commands: `--alias NAME` > `alias.txt`.

Restart `receive` after changing the persisted alias so peers see the new name.

## send

**Always prefer IP** from scan. Use **`--no-scan`** to forbid slow alias lookup.

```bash
lsend scan --json --timeout 5000
lsend send 192.168.1.10 ./file.pdf --json --no-scan
echo "status ok" | lsend send 192.168.1.10 --text --json --no-scan
lsend send 192.168.1.10 --message "status ok" --json --no-scan
```

```json
{
  "command": "send",
  "ok": true,
  "target": { "ip": "192.168.1.10", ... },
  "resolved_via": "ip",
  "kind": "file",
  "files": [{ "name": "file.pdf", "path": "/abs/file.pdf", "size": 1024, "status": "finished" }]
}
```

Message send (`--text`, `--message`, `--clipboard`):

```json
{
  "command": "send",
  "ok": true,
  "target": { "ip": "192.168.1.10", ... },
  "resolved_via": "ip",
  "kind": "message",
  "text": "status ok",
  "size": 9,
  "status": "finished"
}
```

`kind`: `"file"` for path-based transfers, `"message"` for text modes.

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
  "command": "send",
  "ok": false,
  "code": "target_not_found",
  "error": "No device found with alias \"...\". Run `lsend scan --json` first or pass an IP address.",
  "hint": "Run `lsend scan --json` first and use the device IP, or pass an IP address directly."
}
```

## Agent eval checklist

See `lsend agent eval` for a step-by-step smoke test.

## Notes

- Do not run `receive` while another process holds port 53317 (e.g. the LocalSend app or another `lsend receive`)
- **Do not use alternate `--port` for receive** unless you accept that other devices cannot discover this CLI via scan. LocalSend multicast discovery binds UDP and TCP to the same port; the LocalSend app and default `scan` listen on 53317 only.
- On `port_in_use`, prefer closing whatever holds port 53317 (e.g. the LocalSend app, another `lsend receive`) or reusing an existing receiver — do not auto-kill processes or silently switch ports
- Identity (TLS) stored in `~/.config/lsend/`
- Device alias persisted in `alias.txt`; manage with `lsend alias`; `--alias` overrides for one run
- Receive PIN via `receive --pin` (persisted in `receive_pin`) or `LSEND_RECEIVE_PIN`
