---
name: localsend-cli
description: >-
  Send and receive files on the LAN with the headless lsend CLI. Use when
  an agent needs to discover LocalSend peers, transfer files to a device IP,
  or run a one-shot receive server with JSON output.
---

# lsend CLI (agent workflow)

Non-interactive file transfer compatible with the official app.

## Before running

1. Build or locate the binary: `cargo build --release` → `target/release/lsend`
2. Read focused docs offline: `lsend agent` or `lsend agent send`
3. Use **`--json`**, piped stdout, or **`LSEND_NO_TUI=1`** for machine-parseable output
4. Close the official app before `receive` (port 53317 conflict)

## Discover devices

```bash
lsend scan --json --timeout 5000
```

Parse `.devices[].ip` from the JSON object. Prefer IP over alias for send.

## Send files (preferred path)

```bash
lsend send <IP_FROM_SCAN> /path/to/file --json --no-scan
```

- **`--no-scan`** avoids a slow implicit rescan when the target is an alias
- Add **`--pin`** if the receiver sets `LSEND_RECEIVE_PIN`

Success JSON includes `"resolved_via": "ip"` and `"files"[].status`.

## Receive files (automation)

```bash
lsend receive --json --once --dir /tmp/lsend-inbox
lsend receive --json --once --pin 123456 --dir /tmp/lsend-inbox
```

Stdout is NDJSON (`ready` → `file_saved` → `transfer_complete` → `shutdown`).
Use **`--once`** so the process exits after one transfer.
Receive PIN: **`--pin`** (persisted to `receive_pin`) > config file > `LSEND_RECEIVE_PIN` env.

## Errors

Check exit code and JSON envelope:

| Code | Meaning |
|------|---------|
| 0 | Success |
| 1 | General error |
| 2 | Target not found / no files |
| 3 | Port 53317 in use |

Failure JSON: `{"ok":false,"command":"...","code":"port_in_use","error":"...","hint":"..."}`

## More detail

Repository files: `AGENTS.md`, `lsend agent errors`, `lsend agent eval`
