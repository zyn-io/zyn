# ZCaveDAO members' chat

The chat at **cavebro.win/chat**, for holders of a Collection Z item (DECISIONS §183).

- **Server:** ejabberd 26.09 (`ghcr.io/processone/ejabberd:26.09`) in Docker on Contabo, as `zyn-chat`, its ports on
  the host's loopback only; nginx carries `wss://chat.cavebro.win/ws` to it and `https://chat.cavebro.win/auth` to the gate.
  No federation, no sign-up: `auth_method: jwt`, and only the gate signs logins. Config: `ejabberd.yml` here →
  `/var/lib/zyn/chat/conf/ejabberd.yml`. Signing key: `/var/lib/zyn/chat/conf/jwt.jwk` (made on the box, never copied).
- **Encryption, enforced:** `mod_cave_omemo_only.erl` refuses any room or direct message with words in it that is not
  OMEMO encrypted, before it is relayed or archived. Installed from source with ejabberd's own installer:
  `/var/lib/zyn/chat/modules/sources/mod_cave_omemo_only/{src/mod_cave_omemo_only.erl, mod_cave_omemo_only.spec}`, then
  `docker exec zyn-chat ejabberdctl module_install mod_cave_omemo_only` (or `module_upgrade` after a change).
- **The door:** `zyn-chat-gate` (zynzapd) — a member pass in, a login out; every five minutes it takes out anyone whose
  item moved. Rooms: General, Diggers, Lookouts, Lanterns.
- **The client:** Converse.js 14.0.0 (MPL-2.0), pinned in `apps/cave-collection-z`, served from cavebro.win/converse/.

What the team, which runs it, can see: who is in which room and when, and which chat name came from which pass (the
operator can link a pass to an item). Not the words.
