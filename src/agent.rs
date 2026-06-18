use crate::cli::AgentCommand;

pub fn print(topic: Option<AgentCommand>) {
    match topic {
        None => print_overview(),
        Some(AgentCommand::Scan) => print_scan(),
        Some(AgentCommand::Send) => print_send(),
        Some(AgentCommand::Receive) => print_receive(),
        Some(AgentCommand::Errors) => print_errors(),
        Some(AgentCommand::Eval) => print_eval(),
    }
}

fn print_overview() {
    println!(
        r#"lsend-cli — agent integration guide (progressive disclosure)

Run `lsend agent <topic>` for focused instructions:

  scan      Discover devices on the LAN
  send      Send files to a device
  receive   Accept incoming files
  errors    Exit codes and error JSON schema
  eval      Smoke-test checklist for agents

Global flags for automation:
  --json           Structured stdout (required for parsing; auto when piped)
  --quiet          Suppress human progress text (implied by --json)
  --verbose / -v   Diagnostics on stderr only
  LSEND_NO_TUI=1   Force JSON stdout (same as --json)

Quick start:
  lsend scan --json --timeout 5000
  lsend send <IP> ./file.pdf --json
  lsend receive --json --once --dir /tmp/inbox

Full reference: AGENTS.md in the repository root.
"#
    );
}

fn print_scan() {
    println!(
        r#"## scan — discover LocalSend devices

Command:
  lsend scan --json [--timeout MS]

Defaults:
  timeout: 1500 ms (multicast wait is at least 3500 ms internally)

Success stdout (single JSON object):
  {{
    "command": "scan",
    "ok": true,
    "timeout_ms": 1500,
    "devices": [
      {{
        "alias": "Cute Apple",
        "ip": "192.168.30.162",
        "port": 53317,
        "fingerprint": "...",
        "https": true,
        "version": "2.1",
        "device_type": "desktop",
        "device_model": "macOS"
      }}
    ]
  }}

Notes:
  - Empty devices[] with ok:true means no peers were found (not an error).
  - Prefer device.ip for subsequent send commands (avoid alias rescan).
  - Do not run scan while debugging with -v unless you need stderr logs.
"#
    );
}

fn print_send() {
    println!(
        r#"## send — transfer files to a device

 Command:
  lsend send <TARGET> [FILE...] --json [--pin PIN] [--no-scan]
  lsend send <TARGET> --text --json [--no-scan]          # stdin pipe
  lsend send <TARGET> --message "..." --json [--no-scan]
  lsend send <TARGET> --clipboard --json [--no-scan]

TARGET:
  - IP address (recommended for agents): no network scan, fast
  - Alias: triggers a full scan unless --no-scan is set

Recommended agent workflow:
  1. DEVICES=$(lsend scan --json --timeout 5000)
  2. Pick .devices[].ip from JSON
  3. lsend send "$IP" /path/to/file --json --no-scan

Text / automation:
  echo "hello" | lsend send "$IP" --text --json --no-scan
  lsend send "$IP" --message "hello" --json --no-scan
  lsend send "$IP" --clipboard --json --no-scan

Success stdout:
  {{
    "command": "send",
    "ok": true,
    "target": {{ "alias": "...", "ip": "...", "port": 53317, ... }},
    "resolved_via": "ip",
    "files": [
      {{ "name": "file.pdf", "path": "/abs/file.pdf", "size": 1024, "status": "sent" }}
    ]
  }}

resolved_via:
  - "ip"    — target was an IP address (preferred)
  - "scan"  — alias required a discovery scan

Flags:
  --no-scan   Fail if TARGET is not an IP (prevents slow implicit scan)
  --pin PIN   Required when the receiver uses LSEND_RECEIVE_PIN
  --text      Read UTF-8 from stdin (pipe required; do not use on a TTY)
  --message   Send inline text as a .txt file (text/plain)
  --clipboard Send plain text from the system clipboard
"#
    );
}

fn print_receive() {
    println!(
        r#"## receive — accept incoming files

Command:
  lsend receive --json --once [--dir PATH] [--pin PIN]

Always use --once for agents so the process exits after the first transfer.

Stdout is NDJSON (one JSON object per line):
  {{"event":"ready","alias":"...","port":53317,"https":true,"receive_dir":"..."}}
  {{"event":"transfer_started","sender_alias":"...","file_count":2}}
  {{"event":"file_saved","path":"...","file_name":"...","size":4096}}
  {{"event":"transfer_complete"}}
  {{"event":"shutdown"}}

Receive PIN (sender must pass --pin when sending to you):
  lsend receive --json --once --pin 123456

PIN priority: --pin (saved to receive_pin) > config file > LSEND_RECEIVE_PIN env.

Important:
  - Port is checked before bind; port_in_use errors include a hint with remediation.
  - Close the official app before receive (port 53317 conflict).
  - Do not use alternate --port for receive: discovery uses the same UDP/TCP port, so the official LocalSend app and default scan (53317) will not see this device.
  - On port_in_use: close the official app or reuse an existing receiver; do not auto-kill processes or silently switch ports.
  - Without --once the process runs until Ctrl+C (avoid for agents).
  - JSON mode is auto-enabled when stdout is piped or LSEND_NO_TUI=1.
"#
    );
}

fn print_errors() {
    println!(
        r#"## errors — exit codes and JSON failures

Exit codes:
  0  success
  1  general error
  2  not found (unknown alias, no files, etc.)
  3  port already in use (official app or another lsend instance)

Failure stdout with --json:
  {{
    "ok": false,
    "command": "send",
    "code": "target_not_found",
    "error": "No device found with alias \"...\".",
    "hint": "Run `lsend scan --json` first and use the device IP, or pass an IP address directly."
  }}

Error codes (code field):
  port_in_use       — bind failed (usually 53317); hint explains discovery impact of alternate ports
  target_not_found  — alias not found (use scan + IP, or drop --no-scan)
  no_files          — send called with no paths and no --text/--message/--clipboard
  error             — other failures

Human mode prints "Error: ..." to stderr; JSON mode prints only the envelope to stdout.
"#
    );
}

fn print_eval() {
    println!(
        r#"## eval — agent smoke test

Run these in order from the built binary (adjust paths as needed):

1. Help / agent docs load
   lsend agent
   lsend agent scan

2. Discovery (requires a peer running the official app on the LAN)
   lsend scan --json --timeout 5000
   Expect: ok=true and devices[] (may be empty if no peers)

3. Send (requires a reachable peer IP from step 2)
   lsend send <IP> ./README.md --json --no-scan
   Expect: ok=true, files[].status == "sent"

4. Receive (stop the official app on this machine first)
   lsend receive --json --once --dir /tmp/lsend-inbox
   Expect NDJSON: ready -> ... -> shutdown after a peer sends a file

Pass criteria:
  - Exit code 0 on success paths
  - stdout is valid JSON / NDJSON when --json is set
  - No interactive prompts at any step
"#
    );
}
