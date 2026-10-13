# Roles and moderation: implementation plan

> For agentic workers: REQUIRED SUB-SKILL: superpowers:executing-plans. Steps use `- [ ]`.

**Goal:** an owner with every power, roles with permissions given by the owner,
and den-enforced moderation (slow mode, posting rights, timeout, kick, ban,
deletion, audit), with a page to run it.

**Architecture:** a new `Den.Mod` module owns roles, sanctions, settings and the
audit log in SQLite; channels ask it before relaying. Removal commits in open
rooms are checked against it (the NIF reports removed keys). Kicks and bans
are carried out by members' pages through the existing `expired` broadcast.
The page keeps the den's moderation state (`den.mod`) and draws it.

**Tech stack:** Elixir/Phoenix + Ecto SQLite, Rustler NIF (OpenMLS
PublicGroup), plain JS modules, Playwright e2e.

**Spec:** `apps/cave-den/docs/specs/2026-10-14-roles-and-moderation.md`

## Global constraints

- No owner set: the den behaves as today (any member makes 3 open rooms a
  day, slow mode off), except removal commits in open rooms are checked.
- The den never stores message senders; deletion is by `seq`, non-commits only.
- Unlinkability stays: no key↔pass link; bans are by device key.
- Copy: plain labels, sentence case, Discord's words where they exist
  (Timeout, Kick, Ban, Slow mode, Roles, Server settings is "Den settings").
- Commit only own paths (other sessions share the tree).

## Review focus

1. A moderator acting on an equal or higher role, or giving a role at/above
   their own, or a permission they lack: refused.
2. Owner device offline / not admitted today when `epeaky owner m-…` runs:
   says so, does not guess among several matches.
3. Slow mode with reactions right after a message: the reaction goes through
   (budget 4 per window); the composer countdown survives a room switch.
4. A banned device that still has a session: its sockets drop at once and
   it cannot reconnect; members' pages remove it from open rooms.
5. Deleting a message that is a commit, or in another room: refused.

## Tasks

### Task 1: Den.Mod core (storage, permissions, hierarchy, sanctions, audit)
Files: `server/priv/repo/migrations/20261015000000_den_mod.exs`,
`server/lib/den/mod.ex`, `server/test/den/mod_test.exs`.
Produces: `Den.Mod.owner/0`, `set_owner/1`, `perms/1 :: MapSet`,
`can?(key, perm)`, `top/1`, `above?(actor, target)`, `roles/0`, `holders/0`,
`role_save(actor, attrs)`, `role_delete(actor, id)`, `role_give(actor, key, id)`,
`role_take(actor, key, id)`, `timeout(actor, key, secs, reason)`,
`untimeout(actor, key)`, `ban(actor, key, reason)`, `unban(actor, key)`,
`kick(actor, key, reason)`, `sanction(key) :: nil | %{kind, until}`,
`muted?(key)`, `banned?(key)`, `kicked?(key)`, `public/0` (roles, holders,
sanctions for pages), `audit(n)`, `log(actor, action, target, reason)`,
`settings/0`, `setting_save(actor, map)`, `room_save(actor, id, attrs)`.
- [ ] Tests first: permissions union, owner above all, hierarchy, cannot give
  role at/above own, cannot grant perms not held, timeout expiry, ban, audit rows.
- [ ] Implement; `mix test test/den/mod_test.exs` green; commit.

### Task 2: Removal commits checked
Files: `server/native/den_public/src/lib.rs` (commit returns removed keys),
`server/lib/den/public.ex`, `server/lib/den/rooms.ex` (`relay/3` policy),
`server/test/den_web/room_channel_test.exs`.
- [ ] Test: A's remove of B (fixture remove.bin) refused while B is live and A
  has no Kick; allowed when B's day ended, B is kicked/banned, or A may Kick.
- [ ] NIF returns `{state, epoch, signer, removed}`; policy in open rooms only.
- [ ] Green; commit.

### Task 3: Enforcement on channels and the door
Files: `room_channel.ex`, `voice_channel.ex`, `user_socket.ex`, `door.ex`,
`rooms.ex` (`at`, `delete_message/2`), tests.
- [ ] Tests: timed-out device's words refused, its commit taken; post roles;
  slow budget (4 per window, bypass); typing dropped while timed out; voice
  join refused; banned socket refused and door enter refused; `at` on msg and
  history; `delete` by seq with Manage messages, refused for commits.
- [ ] Implement; green; commit.

### Task 4: Lobby protocol and broadcasts
Files: `lobby_channel.ex`, `rooms.ex` (room settings columns in `listed`),
`test/den_web/lobby_test.exs`.
- [ ] join replies `mod` (public state + own perms); events role_save,
  role_delete, role_give, role_take, timeout, untimeout, kick, ban, unban, bans,
  room_save, room_delete, rooms_order, settings_save, audit, invite_make,
  invites, invite_withdraw, requests, approve, decline, owner_pass; each
  broadcasts `mod` / `room` / `settings`; kick/ban broadcast `expired` to
  rooms and disconnect the device. open_room needs Manage channels once an
  owner exists.
- [ ] Tests per event (allowed, refused); green; commit.

### Task 5: Host commands
Files: `host.ex`, `scripts/den/root/install.sh` (epeaky wrapper), tests.
- [ ] `epeaky owner [m-xxxxxx|--clear]`, `epeaky audit [N]`; prefix match over
  admitted devices; ambiguous → asks for more characters.
- [ ] Tests; commit.

### Task 6: Den settings (name, description, icon, limits)
Files: `mod.ex`, `limits.ex` (override ≤ env), `door_controller.ex` (ways +
`/door/icon`), `router.ex`, tests.
- [ ] Tests: ways shows name; icon served with its type, ≤256 KB png/jpeg/webp;
  limits override capped by env; green; commit.

### Task 7: den.js
Files: `web/den.js`, `web/tests/den.test.js`.
- [ ] `den.mod` state from lobby join + `mod` events; `can(perm)`,
  `roleOf(key)`, `colourOf(key)`; slow mode: `slowLeft(room)` from last own
  message, `send` refuses early with the seconds left; receivers hide a
  second msg/file from one key inside the window by the den's `at`; `deleted`
  removes by seq; @everyone counted as a mention only from holders of
  Mention everyone; admin calls (`modDo(event, payload)`).
- [ ] Unit tests for the hide rule and @everyone; commit.

### Task 8: The page
Files: `web/den-ui.js`, `web/den.css`, `web/index.html`.
- [ ] Role colour on names; members grouped by shown roles, then others; a
  member menu (Timeout, Kick, Ban, Roles) for those allowed; Delete on any
  message for Manage messages; slow mode line and countdown; "You do not have
  permission to send messages in this channel."; den name in the header;
  Den settings dialog with sections Overview, Roles, Channels, Members, Bans,
  Invites, Audit log.
- [ ] e2e (Task 9) green; commit.

### Task 9: e2e
Files: `web/tests/den.e2e.mjs`.
- [ ] Owner by command, Moderator role (Timeout, Manage messages), moderator
  times out a member, deletes a message, cannot ban; slow mode countdown and a
  forced fast send hidden for others; ban removes from General and the door
  refuses it; announcements channel.

### Task 10: Docs, publish, deploy
- [ ] README (moderation section, `epeaky owner`), THREAT_MODEL, DEVLOG.
- [ ] Publish (scripts/den/sync.sh --push), CI green, `epeaky update` on the
  community server; load test with slow mode on.
