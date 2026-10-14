# Audit scope

Epeaky has had one internal review and no independent audit. This is what an
auditor would look at, in order, and how to run it. The claims to test are
in [THREAT_MODEL.md](THREAT_MODEL.md); anything it says the server cannot do
is in scope.

## What, by priority

1. **The MLS core** (`mls/src/lib.rs`, about 1,000 lines of Rust over
   OpenMLS 0.9, compiled to wasm). Group creation, Add and Welcome, external
   joins to open rooms, commits, removing members, the voice key export, the
   blind-token client side. Does the den, holding every commit in plaintext,
   learn more than THREAT_MODEL says? Can a member or the den put a device
   into a group it should not be in, or keep one in?
2. **The server's checks of public state** (`server/native/den_public`, about
   2,700 lines of Rust; `server/lib/den/rooms.ex`, `mls.ex`). The den checks
   each commit against a room's public state and orders them. Can a member
   get a commit accepted that others cannot apply, and use the undo
   (`report`, `undo_unannounced`) to take members out or roll a room back?
3. **The door** (`server/lib/den/door.ex`, `gate/`, `keys.ex`). Blind RSA
   (RFC 9474) day tokens: can the den link a token to the pass that asked for
   it beyond timing and addresses? Three tokens a pass a day: can it be
   beaten? Day keys published ahead: can the den show one member a different
   key without the page's key check noticing?
4. **The page** (`web/den.js`, about 1,900 lines; `den-ui.js`, `den-admin.js`).
   What it takes from other members and shows: names, profiles, links
   between devices, agent claims, pins, files, reactions, edits. XSS in the
   page is the worst case here, since the keys are in it. The content
   security policy (`server/lib/den_web/page.ex`).
5. **Files and voice** (`web/den.js` files section, `voice-worker.js`,
   `server/lib/den/voice.ex`). Chunked AES-GCM files; per-frame voice
   encryption with a key from the MLS exporter; replay counters.
6. **Agents** (`agent/`, about 850 lines). Owner confirmation by hashed code,
   claims newest first, the kind printed on each line, private rooms only
   with the owner. Can a member make an agent treat their words as its
   owner's? The local socket or named pipe, the lock, the folder's files.
7. **Pushes** (`server/lib/den/push.ex`). RFC 8291 encryption, RFC 8292
   signing, the allowed push services (no server-side request to an address
   of a member's choosing), rate limits.

## Known and accepted

These are written down in THREAT_MODEL and are not findings: the den sees
membership and timing; the door and the den share a machine; the den serves
the page that holds the keys (a dishonest host can change it); a new member
cannot read what came before it joined.

## Running it

```sh
mls/build.sh && (cd mls && cargo test)
cd server && mix deps.get && MIX_ENV=test mix test
cd web && npm test                 # the page's own tests
cd agent && ./build.sh && npm test
```

End-to-end, with a dev den on :4000 (`cd server && mix phx.server`) and the
page built into it (`web/build.sh`): `node web/tests/den.e2e.mjs` and
`node agent/tests/agent.e2e.mjs` (Chrome and playwright-core needed).

## Reporting

As in [SECURITY.md](SECURITY.md): privately, before anything is published.
