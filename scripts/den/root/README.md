# Epeaky

A self-hosted chat for small private circles, with end-to-end encrypted
rooms, files and voice. Each Epeaky server is a **den**. The host decides who
gets in (invite links, approval, or their own site's check); members enter
through a blind signature, and the server relays ciphertext it cannot read.

![A den: rooms on the left, a conversation in General](docs/screenshot.png)

**Status: alpha.** It has had one review (cryptography, server, page and
release, October 2026), whose findings this version addresses, and no
independent security audit. Read [THREAT_MODEL.md](THREAT_MODEL.md) before
you trust it with anything that matters.

- **Rooms** are MLS groups (RFC 9420, OpenMLS), held only on members'
  devices. Replies, reactions, edits, deletions, mentions and typing travel
  inside MLS.
- **Files and images** are encrypted in the browser and stored as opaque
  blobs.
- **Voice rooms** forward audio the server cannot hear: each frame is
  encrypted in the browser under a key from the room's MLS group.
- **Entry** is a blind token (RFC 9474): the door checks a pass and signs a
  token it never sees; the den admits the device that token names.

[THREAT_MODEL.md](THREAT_MODEL.md) says exactly what the server can and
cannot see. Read it before you run a den for people who depend on it.

## Install

On a Linux server (x86-64) with a domain pointing at it, TCP 80 and 443 open,
and UDP 50000–50199 open for voice:

```sh
curl -fsSL https://epeaky.zyn-network.com/install.sh | sudo bash
# or: sudo bash install.sh --domain chat.example.org [--email you@example.org] [--yes]
```

It installs Docker if it is missing (after asking), puts the den and Caddy
(TLS from Let's Encrypt) in `/opt/epeaky`, starts them, and prints your first
invite link: open it on the device you will use. Then, on the server:

```sh
epeaky invite 5 7      # 5 one-use invite links, good for 7 days
epeaky requests        # people asking to join, once approval is open
epeaky approve <id>    # or: epeaky decline <id>
epeaky members         # how many, by the way they joined
epeaky update          # the latest image
```

Settings are in `/opt/epeaky/.env` (the ways in, the rooms; then
`docker compose up -d` there). Voice offers devices the server's own public
addresses: on a host whose public address is not on the machine itself (AWS
and GCP put it behind NAT), set `DEN_VOICE_PUBLIC_ONLY=0`, and expect voice to
work only for devices that can reach that private address.

## Back up and update

What a den keeps is in its data folder (`/opt/epeaky/data` with the
installer): the database `den.db`, the day keys in `keys/`, the files in
`blobs/` and the release cookie. To back up while it runs:

```sh
cd /opt/epeaky/data
sqlite3 den.db "VACUUM INTO '/root/den-$(date +%F).db'"   # a consistent copy
tar -czf /root/den-keys-$(date +%F).tgz keys cookie       # keep private: they let in and sign
```

Keep backups no longer than the den keeps ciphertext (30 days by default).
Restoring an older database puts every room behind its members' devices:
they join open rooms again by themselves, and private rooms need their
members invited again. `epeaky update` pulls the latest image and restarts
it; the database is migrated on start.

## Layout

| Path | What it is |
|---|---|
| `mls/` | Rust → WebAssembly: MLS rooms, blind tokens, voice keys. `cargo test` |
| `server/` | Elixir / Phoenix: the door, the relay, files and voice. `mix test` |
| `server/native/` | Rust NIF: each room's public MLS state, so the server can check commits |
| `web/` | The page the server serves at `/`: rooms, messages, files, voice. `npm test` |

## Build

You need Rust with the `wasm32-unknown-unknown` target, `wasm-bindgen-cli`
0.2.128 exactly, Elixir 1.19 on OTP 28, and libsrtp2, OpenSSL and pkg-config
for voice (`brew install srtp openssl pkg-config` on macOS). The server's
build compiles its Rust NIF, so it needs Rust too.

```sh
rustup target add wasm32-unknown-unknown
cargo install wasm-bindgen-cli --version 0.2.128
(cd mls && ./build.sh)                   # → mls/pkg/
(cd server && mix setup && mix test)
web/build.sh                             # the page → server/priv/page/
(cd web && npm test)
# In a browser, against a dev den (cd server && mix phx.server):
(cd web && npm install && npm run e2e)
```

## Run

The `Dockerfile` at the root builds the whole den, the wasm and the page
included: `docker build -t epeaky .` (the image `ghcr.io/zyn-io/epeaky` is
built the same way). `server/Dockerfile` builds only the server, around a page
already built into `server/priv/page`. Run it bound to loopback behind a TLS
proxy that sends `X-Forwarded-Proto: https`; the den forces https and
redirects every request that arrives without it.

| Variable | |
|---|---|
| `SECRET_KEY_BASE` | Phoenix's secret (`mix phx.gen.secret`) |
| `PHX_HOST` | the den's public host name |
| `PORT` | the port it listens on (default 4000) |
| `DEN_IP` | the address it binds to (default 127.0.0.1) |
| `DATABASE_PATH` | the SQLite file |
| `DEN_KEYS_DIR` | where the day keys are kept (private; back it up) |
| `DEN_ORIGINS` | other sites' pages allowed to use it, comma-separated (its own page always may) |
| `DEN_PAGE_CONNECT` | origins its page may fetch from besides the den, such as where the day keys are published |
| `DEN_ENTRY` | the ways in: `invite`, `approval`, `external` (default `invite`) |
| `DEN_ISSUER_TOKEN` | the host site's token for registering passes (`external`; 32+ characters) |
| `DEN_ROOMS` | the rooms every member sees, as `id:Title,…` (default `general:General`) |
| `DEN_GATE` | another gate module, compiled in from `extra/` |
| `DEN_EXTRA_CHILDREN` | processes an add-on starts, by module name |
| `DEN_KEYS_AHEAD` | days of day keys the den keeps ahead itself (default 45; 0: the host makes them) |
| `DEN_VOICE_PUBLIC_ONLY` | `1`: voice offers devices only public addresses |
| `POOL_SIZE` | database connections |

**Limits** go under `config :den, :limits`, each key optional. They are per
device per day unless noted:

| Key | Default | |
|---|---|---|
| `message_bytes` | 20 MB | bytes of messages relayed |
| `file_bytes` | 250 MB | bytes of files stored |
| `files` | 200 | files stored |
| `invites` | 200 | invitations sent |
| `inbox` | 100 | invitations waiting for one device at once |
| `history_bytes` | 4 MB | ciphertext in one history reply |
| `min_free_bytes` | 2 GB | free disk below which nothing more is taken |

Ciphertext and files are kept `retention_days` (30) days.

Voice needs UDP ports 50000–50199 open to the internet. There is no TURN
server option yet, so a member behind a strict NAT may not get audio.

**Day keys.** The door signs with one key per UTC day. The den keeps them
itself: it makes 45 days ahead (`DEN_KEYS_AHEAD`), deletes those it can no
longer use, and publishes the next 31 at `/door/keys`, the same for everyone,
which the page reads by default. A host who would rather publish keys from
somewhere else sets `DEN_KEYS_AHEAD=0`, makes them with
`bin/den eval 'Den.Release.keys(90, "den-keys.json")'`, publishes that file,
and points the page's `keysUrl` at it.

## The page

The den serves its page at `/` from `server/priv/page`, built by
`web/build.sh`. The page runs only its own scripts and WebAssembly, under a
policy that allows no inline or browser-extension scripts, because it holds
the device's keys. Serve nothing else from the den's origin.

To make it yours, put an add-on directory on top: `web/build.sh OUT ADDON`
runs `ADDON/build.sh OUT`, which can replace the page (`index.html`), add
styles, and write `config.json`:

| Key | |
|---|---|
| `keysUrl` | where the day keys are published (default `/door/keys`, on the den) |
| `prefix` | the name this browser keeps the den's state under |
| `issuer` | the page that lets members in (below) |
| `issuerCode` | `true` if the issuer makes the pass code and sends it back (otherwise it gets only a hash) |

## Who gets in

The den keeps its own member list. Each member's device makes a pass code and
sends the den only its hash, so the list holds no codes. The host chooses
which ways in are open with `DEN_ENTRY` (comma-separated; `invite` alone by
default):

- **`invite`: invite links.** The host makes one-use links that expire. The
  code rides after `#`, so it never reaches a server log, and the page takes
  it off the address at once.
- **`approval`: the host lets people in.** Someone writes a short note and
  asks; the host approves or declines. The note is forgotten once decided.
- **`external`: the host's own site decides.** A page on the host's site
  (`config.json`'s `issuer`) checks whatever the host requires (a wallet, a
  sign-in, a token). The den's page opens it in a window, having made the
  member's pass code itself, and gives it only the code's hash after `#`.
  The issuer registers that hash with `DEN_ISSUER_TOKEN` and answers the
  page; it never learns the code:

  ```js
  const pass = new URLSearchParams(location.hash.slice(1)).get('den-pass');
  // …check the visitor, then from the host's server:
  //   POST https://den.example.org/door/passes  {"pass": pass}
  //   authorization: Bearer $DEN_ISSUER_TOKEN
  opener.postMessage({ den: 'pass', registered: true }, 'https://den.example.org');
  ```

  The page takes the answer only from that window and the issuer's origin.
  Have the issuer ask its visitor to confirm ("let this device into the
  den") before it registers anything: any site can open the issuer with a
  hash of its own choosing. A host whose check must make the code itself
  (a wallet that derives it, say) sets `"issuerCode": true` and answers
  `{ den: 'pass', code }`; then the issuer knows the code. The host's
  wallet or sign-in scripts run on the issuer's page, never next to the
  den's keys. `DELETE /door/passes/<hash>` removes a pass.

The host runs these on the den's server:

```sh
bin/den rpc 'Den.Host.invite(5, 7)'           # 5 one-use links, good for 7 days
bin/den rpc 'Den.Host.invites()'              # made, used, withdrawn
bin/den rpc 'Den.Host.withdraw("3fa9c1d2")'   # an invite, and the pass it let in
bin/den rpc 'Den.Host.requests()'             # people asking to join
bin/den rpc 'Den.Host.approve("<id>")'        # or Den.Host.decline("<id>")
bin/den rpc 'Den.Host.members()'              # how many, by the way they joined
```

A device keeps its pass code and gets its next days' tokens itself, so a
member enters once and stays in until the host removes the pass. Removing it
ends entry at the end of that day.

**Your own gate.** To decide membership another way (holders of a token on
some chain, say), write a module implementing `Den.Gate` (`id/1` and
`live?/1`), put it in `server/extra/lib`, and set `DEN_GATE` to its name. The
den compiles `extra/` in when the folder is there; its tests go in
`extra/test`. Processes your add-on needs start with `DEN_EXTRA_CHILDREN`.
Whatever the gate, the door's own rules stay: three day tokens per pass per
day, blind signatures, and a device's proof of its key at entry.

## License

Licensed under either of the [Apache License, Version 2.0](LICENSE-APACHE) or
the [MIT license](LICENSE-MIT), at your option. Third-party terms are in
[NOTICE](NOTICE).
