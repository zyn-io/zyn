# epeaky-host-mcp

Set up and run an Epeaky den on a server you already have, from any MCP
client (Claude Desktop, Claude Code, others). It works over **your own SSH**:
it holds no keys, keeps nothing, and never creates, rents or buys a server.

```sh
claude mcp add epeaky-host -- npx -y epeaky-host-mcp
```

or, in any MCP client's config:

```json
{ "command": "npx", "args": ["-y", "epeaky-host-mcp"] }
```

Your SSH key must already reach the server (`ssh user@server` works without
a password). Node 22 or later.

This is apart from [`epeaky-mcp`](../mcp/README.md) on purpose: that one is
an agent in a den's rooms; this one has root on the den's server. Give each
to an agent knowingly.

## Tools

| Tool | Changes the server | What it does |
|---|---|---|
| `host_check` | no | the system, memory, disk, sudo, ports 80/443, Docker, whether the domain points at the server (from a public resolver), NAT and the firewall, as what is fine and what to fix |
| `host_install` | yes | Docker if missing, the den and Caddy in `/opt/epeaky`, a certificate, the firewall's ports; returns the first invite |
| `host_status` | no | owner, members, requests to join, invites |
| `host_logs` | no | the den's last log lines |
| `host_owner` | yes | make a device (`m-…`, as its page shows it) the owner |
| `host_invite` | yes | one-use invite links |
| `host_withdraw` | yes | withdraw an invite and the pass it let in |
| `host_approve`, `host_decline` | yes | answer a request to join |
| `host_setting` | yes | change a known setting in `/opt/epeaky/.env` (backed up first), then restart |
| `host_update` | yes | the latest Epeaky |

Tools that change the server are marked so (`readOnlyHint: false`), so your
client asks you before each call. Invite links come back with an instruction
to show them only to you.

## What keeps it safe

- **Every value is checked to a strict shape** before it reaches ssh: the
  server (`user@host`), the domain, ids (hex), device names, counts, and each
  setting by its own rule. ssh hands its command to the server's shell as one
  string, so this check, not argument lists, is what keeps a value from
  becoming a command; values are single-quoted as well.
- **Only these settings**: `DEN_ENTRY`, `DEN_ROOMS`, `DEN_DONATE`,
  `DEN_DONATE_LABEL`, `DEN_ISSUER_LABEL`, `DEN_SCREEN`, `DEN_SCREEN_BITRATE`,
  `DEN_RETENTION_DAYS`, `DEN_LIMITS`, `DEN_PUBLIC_IP`. Secrets and the
  domain are never touched.
- **No passwords.** If sudo asks for one, the tool returns the exact command
  for you to run yourself.
- ssh runs in batch mode and accepts a server's host key the first time it
  sees it (`StrictHostKeyChecking=accept-new`), as a first manual `ssh`
  would after you said yes; a changed key is refused.

`EPEAKY_HOST_SSH` points at another ssh.

## Tests

```sh
npm test
```

They check the tool list (nothing makes a server; what changes is marked),
that bad values never reach ssh, the commands each tool runs, the settings
rules, the check's report, and that every `epeaky` command and setting used
exists in the installer and the den.
