# The den — ZCaveDAO's members' chat

At **cavebro.win/chat**, for holders of a Collection Z item (DECISIONS §184).

- `mls/` — Rust → wasm (`./build.sh`): OpenMLS rooms on members' devices, and RFC 9474 blind tokens. `cargo test` runs
  a room's life, a refused commit and the token round trip.
- `server/` — Elixir / Phoenix (`mix test`): the door (`/door/keys`, `/door/sign`, `/door/enter`) and the relay
  (`/socket`, channels `room:<id>`). It orders and stores ciphertext and never holds a key.
- The page: `apps/cave-collection-z/den.js` and `chat.html`.

**Who sees what.** The door sees a member pass and a blinded token, never the token. The den sees a device key, its
rooms and when it writes, never the pass. A device fetches tomorrow's token today, so from its second day the den
cannot time the two together. Words are readable only on members' devices.

**Day keys.** One RSA key per UTC day, made ahead on the server (`bin/den eval 'Den.Release.keys(90, "/var/lib/den/den-keys.json")'`)
and published as `apps/cave-collection-z/den-keys.json` — the same file for everyone. Re-publish before they run out.

**Running it.** `server/Dockerfile` builds a release; on Contabo it runs with the host's network, bound to 127.0.0.1,
behind nginx at chat.cavebro.win, with `DATABASE_PATH`, `SECRET_KEY_BASE`, `PHX_HOST`, `DEN_NODE_URL`, `DEN_KEYS_DIR`
and `DEN_ORIGINS` from its env file.
