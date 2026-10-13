# Roles and moderation (Epeaky, project 1 of 4)

Status: draft for review, 14 Oct 2026.

## Why

A den run as a community chat (first: a gated community of up to ~200 people
on a 2-core server) needs what a Discord server has: one owner with every
power, who can hand parts of it to others, and tools that stop floods,
trolls and spam. Today a den has none: the host has only the command line
(`epeaky invite`, `withdraw`, `approve`), and every member is equal.

This project is the den side plus the smallest page changes needed to use
it. The settings screens (project 2), the Discord-style layout (project 3)
and the Solana burn door (project 4) build on it.

## What the den can and cannot do

The den never reads messages. It knows which device sent each ciphertext,
when, in which room, and whether it is a commit (MLS PublicMessage) or an
application message (PrivateMessage). Moderation is built only on that:

- It can refuse what a device sends (timeout, ban, slow mode, posting rights).
- It can drop a stored ciphertext by its `seq` and tell everyone to hide it
  (deleting someone else's message), without seeing it.
- It can refuse a device at the door and on every channel (ban).
- It cannot remove a device from a room's MLS group itself (it holds no
  member keys). A member's page does that by a Remove commit; the den decides
  whose removals it accepts.

## Identity

A role or sanction belongs to a **device key** (the MLS signature key the den
already authenticates on every socket). A person with linked devices has
several; the page that grants a role or a sanction applies it to every device
it knows as that person's (link claims live in MLS, so only pages know them).
The den does not learn which devices are one person beyond what those grants
reveal.

## Owner

- Exactly one owner device. Made by the host: `epeaky owner m-xxxxxx` (the
  six characters the page shows; the den finds the one member key with that
  prefix, or asks for more characters if several match).
- `epeaky owner` shows the owner; `epeaky owner --clear` removes it.
- The owner can pass ownership to another device from the page (two-step
  confirmation). The host keeps the final say through the command.
- The owner has every permission, is above every role, and cannot be timed
  out, kicked or banned.

## Roles and permissions

Discord's model, cut to what a den can enforce:

- A **role** has a name, a colour, a position (order), whether it is shown
  separately in the member list, and a set of permissions.
- Every device has the implicit **@everyone** role. Its permissions are the
  den's defaults (by default: create invites off, everything below off).
- A device's permissions are the union over its roles.
- **Hierarchy**: a device can only act on devices whose highest role is
  below its own highest role, and can only create, edit, give or take roles
  below its own highest role. The owner is above all.

Permissions:

| Permission | Lets a device |
|---|---|
| Administrator | everything below, and edit roles below its own |
| Manage server | change the den's name, icon, description and limits |
| Manage channels | create, rename, reorder and delete open rooms; set slow mode and who may post |
| Manage roles | create and edit roles below its own; give and take them |
| Manage messages | delete anyone's message |
| Timeout members | stop a device from posting, reacting, typing and speaking for a while |
| Kick members | remove a device from every open room (it may come back with its pass) |
| Ban members | remove a device and refuse it at the door and on every channel |
| Manage invites | make and withdraw invite links, approve or decline requests to join |
| Bypass slow mode | post without waiting |
| Mention everyone | use @everyone in an open room |

Not in this project: private channels by role (needs MLS adds by role),
per-channel role overrides beyond "who may post", voice moderation beyond
timeout (server mute in voice comes with project 3).

## Channels (open rooms)

New per-room settings, set by Manage channels:

- **Slow mode**: 0 (off), 5, 10, 15, 30 s, 1, 2, 5, 10 min, 1, 6 h.
- **Who may post**: everyone, or chosen roles (an announcements channel).
- **Topic** (shown in the header).
- Order in the list.

Once the den has an owner, creating open rooms becomes Manage channels only
(today any device may make 3 a day). Private rooms stay as they are: members make them and invite
whom they like.

## Enforcement

### Slow mode

The den cannot tell a message from a reaction, so three parts:

1. **Page**: after sending a message or file, the composer shows "Slow mode:
   23 s" and does not send until the time has passed. Reactions, edits and
   the page's own housekeeping are not held.
2. **Den**: per device per room, at most 4 application messages per slow-mode
   window (a message plus a few reactions); over that, the relay is refused
   ("Slow mode is on"). Commits are never held. Bypass slow mode skips it.
3. **Every page** hides a second message or file from the same device in the
   same room within the window, by the den's own time on each message (the
   den adds `at` to every message it relays and to history). A modified page
   that sends faster gets its extra messages hidden for everyone.

### Who may post

The den refuses application messages from devices without a posting role in
that room (reactions too; acceptable for an announcements channel). Pages
show "You do not have permission to send messages in this channel." in place
of the composer.

### Timeout

Until a set time (60 s, 5 min, 10 min, 1 h, 1 day, 1 week), with an optional
reason: the den refuses the device's application messages in every room,
typing and voice joining. The device keeps reading. Pages show it greyed with
a clock and its remaining time.

### Kick

The den marks the device kicked until its day ends, drops its connections,
and tells every open room it is in to take it out: members' pages remove it
by commit, as they already do for a device whose day ended (the first commit
wins). The den accepts a commit that removes another member from an open room
only when one of these holds:

- the sender may Kick (or Ban) and is above the removed device,
- the removed device's pass has ended (today's housekeeping), or
- the removed device is banned.

This also closes today's gap where any member could remove any other from an
open room. The NIF's `commit` returns the removed keys so the den can check.

A kicked device can come back the next day with its pass (Discord's kick,
where the person may rejoin).

### Ban

The den records the key (with reason and who banned), refuses it at the door
and on every channel, drops its connections, and members' pages remove it
from every open room as for a kick. Private rooms: the den refuses the
device's channel; members decide whether to take it out. Unban removes the
record.

A ban is by device. The den cannot link a device to the pass (or burn) it came
in with: that is what the blind day tokens are for. So a banned person can come
back with a new device from the same pass, as a new member name (at most 3 a
day per pass). Moderators ban that device too; on a den with burn entry each
new pass costs a burn. A host who wants bans that follow the pass would have
to give up that separation; not offered.

### Deleting a message

Manage messages (or the author, as today): the den deletes the stored
ciphertext for that `seq` (never a commit) and broadcasts `deleted {seq}`;
pages hide it and drop their copy. The den does not keep who sent a message,
so deletion does not follow the role order (as on Discord). The den logs who
deleted, not what.

## Audit log

The den keeps, for 90 days: time, actor key, action, target key or room, and
reason. Readable by Administrator, Manage server, and the roles that can take
the actions listed. Shown in project 2's settings; in this project through
`epeaky audit [N]`.

## Den settings

Name, description and icon (a public image, not encrypted), shown on the
door page and in the header. The limits already in `DEN_LIMITS` (file size,
daily bytes, room members) become editable by Manage server; the env file
stays the default and the ceiling for what the page may set (a host can
still cap what moderators choose).

## Protocol

- Lobby channel (all owner and moderator actions, checked against the
  sender's permissions and hierarchy): `roles`, `role_save`, `role_delete`,
  `role_give`, `role_take`, `timeout`, `untimeout`, `ban`, `unban`, `bans`,
  `settings_save`, `room_save`, `room_delete`, `rooms_order`, `audit`,
  `owner_pass`.
- Pushed to everyone: `roles` (all roles, and which keys hold which), the
  room list with settings, sanctions in force (timeouts with their end),
  `settings`. These are public to members: who moderates is visible, as on
  Discord.
- Room channel: `deleted {seq}`; `msg` and `history` gain `at` (unix seconds).
- `/door/ways` gains the den's name, description and icon.

## Storage

New tables: `roles` (id, name, colour, position, hoist, perms), `role_keys`
(role, key), `sanctions` (key, kind timeout|ban, until, by, reason, at),
`audit` (at, actor, action, target, reason), `settings` (key, value). New
columns on `rooms`: slow_seconds, post_roles, topic, position. One owner key
in `settings`.

## Page changes in this project

Just enough to use it before project 2:

- Role colours on names; moderators shown first in the member list.
- A member's menu (click their name): Timeout, Kick, Ban, Give role, for
  those allowed.
- Delete on any message for Manage messages. (Pins come with project 3.)
- The slow mode countdown and the "no permission" composer.
- A plain settings dialog for the owner: roles (name, colour, permissions,
  order), channels (slow mode, who may post, topic), bans. Project 2
  replaces it with the full Discord-style settings.

## Threat model changes

The den now knows who holds which role, who is timed out or banned, and the
moderation log: all metadata, never content. A malicious den could invent
bans or deletions (it could always refuse service); it still cannot read,
forge or alter messages. Removing other members from an open room now needs
permission, which closes a disruption a single troll could cause.

## Tests

- Server: each permission allowed and refused, hierarchy (cannot act on equal
  or higher, cannot give roles at or above own), owner untouchable, slow-mode
  budget and bypass, posting rights, timeout expiry, ban at door and channel,
  removal-commit policy (allowed for Kick, for ended passes, for banned;
  refused otherwise), delete by seq, audit entries.
- Page (e2e): owner made by command; owner makes a Moderator role with
  Timeout and Manage messages; the moderator times out a member (their send
  refused, countdown shown), deletes a message (gone for everyone), cannot
  ban; slow mode countdown and a forced fast send hidden by others; a ban
  removes the device from General and the door refuses it.
- Load: the 200-device run again with slow mode on, to show the cost of the
  checks.

## Rollout

Defaults keep today's behaviour for dens that do not use it (no owner: no
roles, any member makes open rooms as now, slow mode off), except the
removal-commit rule, which applies everywhere. Released as a public image;
the community server updates with `epeaky update` and gets its owner with
`epeaky owner`.
