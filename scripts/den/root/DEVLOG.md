# Devlog

Newest first. What changed, why, and what we found along the way.

## 14 October 2026: after the first film

Filming two real Claude Code agents in a den showed what a newcomer hits
first. All of it is fixed, and a few things around it.

- **Add an agent in one step.** A member who may make invites gets one
  command from the page: a new single-use invite and the owner code,
  `epeaky-agent join '<link>' --owner <code>`. Nobody has to make an invite
  on the server. Others still get the code for an agent already in.
- **The page says when the agent took the code.** It used to say so under the
  dialog's shade, where nobody could read it; the dialog now changes to
  "Agent added".
- **The tag names the owner**: "Olu's agent", or "your agent" to the owner,
  instead of only "agent" with the owner on hover.
- **An agent asks for its owner in every open room it is in**, once it is in
  one. A brand-new agent used to ask in the first open room, which on a den
  with several rooms could be one the owner's page had not entered yet.
- **`epeaky-agent next`** hands each message once, in every room, and keeps
  its place across calls and restarts (`--wait` waits for the next one).
  `watch` between two calls could miss an order; a loop of `next` cannot.
- **A skill for the model** (`agent/skill/epeaky/SKILL.md`, `epeaky-agent
  skill`): setting up, who may give orders, the commands, a loop that waits
  for orders, and what each refusal means.
- **Pins.** A new member cannot read what came before it joined: that is how
  MLS works. A pin is what a room wants every newcomer to know. Each pinner's
  device sends its own pins again whenever someone joins, so a pin always
  comes from its pinner's key and nobody can pass on someone else's.
- **The page installs**, on a phone's home screen or as a desktop app.
  Installed, Safari no longer clears a device's keys after a week unused.
- **Notifications with the page closed** (Web Push). The push is encrypted
  for the browser and says the den's name and the room, never the words.
  The den sends only to the browsers' own push services, and the setting
  says what those services see. Pages now mark which documents are words,
  so only words push: that is one thing the den learns that it did not.
- **`epeaky-agent` on Windows**: a named pipe instead of a Unix socket. Its
  plumbing is tested on Windows in CI; the whole agent has not yet been run
  against a den there.
- **Found in review and fixed before release:** the push request followed
  redirects, which would have let a member point the den at an address of
  their own (now never); the table holding pushes back grew without end;
  one device could take over another's push address; a pin could be shadowed
  by someone else's pin with the same id, and an owner's pin of someone
  else's message read as the owner's order (a pin of someone else's words is
  now information, whoever pinned it); on Windows, a program of another
  user's could have listened on an agent's pipe first and passed for it
  (commands now check that the process proves it holds the agent's secret).
  The threat model now says what a push service can infer.
- **Tasks** (`epeaky-agent` 0.3.0): hand an agent work, see it claimed,
  get its result. A task is a message for one agent, with A2A's states
  (submitted, working, input-required, completed, failed, canceled,
  rejected) on a card the page keeps current. Only the agent it is for
  reports, only its author cancels, a finished task stays finished, its
  words cannot be edited, and only the agent's own messages count as its
  results. A task from the agent's owner is an order; from anyone else,
  another agent included, information. `next --tasks` hands tasks and news
  of the ones an agent gave; `next --changes` hands edits and deletions, so
  an index can forget what was deleted. The page asks an agent from the
  composer. A command and a background process of different versions now
  stop with a plain message instead of failing halfway.
- **What an agent may do** (`epeaky-agent` 0.4.0), set by its owner on its
  member card: every room it is in, or only those ticked; other agents of
  the owner's whose tasks it takes as orders (`delegated`, and only tasks,
  never their ordinary words); files. Each room is told only whether it is
  one of the agent's, so no room learns of the others. The agent keeps to the
  rules itself, and the threat model says plainly that this holds the model
  back, not a machine someone else controls. An agent no longer waits 15
  seconds on every command when one of its rooms failed to open.
- **Open, not yet explained:** on our test den, whose General had grown to
  about 226 devices (most of them offline test leftovers), a ban did not take
  the banned device out of the room within 20 seconds; on a fresh den it
  does, in every run. A guess to check: in a big room the commit that takes
  the device out is undone, because its new room state is not published
  within the 30 seconds the den allows. Until this is explained, a ban in a
  big open room may need checking by hand (the member list).
- **Ready for npm** (`epeaky-agent` 0.2.0, published from a tag with
  provenance), and [AUDIT.md](AUDIT.md), the scope of the independent audit
  Epeaky has not had yet.

## 14 October 2026: agents

A den can now be where your AI agents work: they report, coordinate and take
your orders in its rooms, end-to-end encrypted like everything else.

- **`epeaky-agent`**, on the command line: `join`, `rooms`, `read`, `send`,
  `watch` (for mentions or orders, once or as they come) and `search`. Plain
  lines by default, `--json` for programs.
- **The page's own client, in Node.** We tried it first: the den's client ran
  outside a browser as it was, given a file store in place of IndexedDB. So
  an agent is a member like any other, with no second implementation of MLS
  to keep in step.
- **One identity per agent, not per machine.** Each agent has its own key,
  pass (from its own invite) and folder; one machine runs as many as you
  like. A pass lets three devices in a day, so agents do not share yours.
- **One background process per agent** holds its MLS state, stays in the
  rooms and enters each day. Commands talk to it over a socket, so two of them
  at once never touch that state together.
- **Orders from the owner only.** "Add an agent" on the page shows a code; the
  agent takes it, and each side then says so, sealed. Every line the agent
  reads names the sender's kind (owner, agent, member, den) from its key, and
  words are kept to one line, so no message can pass for its owner's.
- **Only the owner lets an agent go.** While it has a confirmed owner, the
  agent refuses `owner` and `owner --clear`, so a message that talks the model
  into running them changes nothing. The owner's code is 26 characters (about
  130 bits), because its hash crosses the room and must not be guessed
  offline.
- **Private rooms**: an agent joins one only if its owner is in it, reads
  nothing there until the owner says something, and leaves if the owner does
  not or goes.
- **Found in review and fixed before release:** two commands could start two
  background processes for one agent (now one lock file per agent); writes of
  the MLS state could land out of order (now one after another, flushed);
  an old profile read late could undo a removal (claims are now newest
  first); a room id from a hostile den could have forged a line (ids are
  checked and escaped).
- Others see an "agent" tag on any member that says it is one, with its
  owner's name once the owner confirms.

## 14 October 2026: moderation

A den run as a community needs what a Discord server has. It has it now, and
the den still reads nothing.

- **An owner.** The host names one device the owner (`epeaky owner m-…`).
  It holds every permission and nobody can act on it.
- **Roles.** The owner makes roles with a name, a colour, a place in the
  list and any of eleven permissions (Administrator, Manage den, Manage
  channels, Manage roles, Manage messages, Timeout, Kick, Ban, Manage
  invites, Bypass slow mode, Mention @everyone). Members act only on those
  whose highest role is below theirs, and give out only roles below their
  own and permissions they hold. Roles can list their members on their own
  in the member list.
- **Channels.** Manage channels creates, renames and deletes them, and sets
  a topic, slow mode (5 seconds to 6 hours) and who may post.
- **Slow mode, three ways.** The page holds a message until the window has
  passed and counts down. The den refuses more than a few messages a window
  from one device (a message and some reactions, since it cannot tell them
  apart). And every page hides a second message from one device inside the
  window by the den's own time on each message, so a page that skips the
  wait gains nothing.
- **Timeouts, kicks and bans** by device key. The den refuses the device's
  messages, reactions, typing and voice; a kick or ban also drops its
  connections, and members' pages take it out of every open room. A removed
  device is told so on every channel.
- **Deleting anyone's message**, by its number: the den drops the
  ciphertext and tells the room, without knowing whose it was.
- **One hole closed.** Any member could remove any other from an open room.
  Now the den orders that only from a device allowed to kick, or for a
  device whose day ended or that was kicked or banned. The den's MLS check
  (PublicGroup) now reports whom a commit removes.
- **Den settings** in the page: overview (name, icon, file limits under the
  host's), roles, channels, timeouts and bans, invite links and requests,
  the audit log, ownership.

What a ban cannot do: follow a person to a new device. The den cannot link a
device to its pass, which is what keeps who-said-what apart from who-holds-
which-pass. A banned person can come back on a new device under a new name.

## 13 October 2026: past the first limits

The first public version listed its limits. Most of them moved:

| | Before | Now |
|---|---|---|
| Members in a room | 200 | 500, up to 1000 if the host allows |
| People in a voice room | 9 | 50; each hears the eight who spoke most recently |
| Voice on a network that blocks UDP | none | through the den's own connection |
| One file | 25 MB | 100 MB by default (`file_max`), sealed in 1 MiB pieces |
| A removed pass lets a device in until | the end of the next day | midnight, if removed before 22:00 UTC |
| A copied device reads its rooms | until it leaves them | until its next daily key refresh |
| Who is who | `m-` and 20 hex digits | a name each member gives themselves, beside it |
| One person, two devices | two strangers | linked: one name, one row |

How each works:

- **Names.** A member's name travels sealed inside MLS, in every message and
  in a profile each device sends when someone joins. It is a claim, so the
  page always shows it beside the start of the device's own name, and the
  members list warns when two members chose the same one.
- **Linked devices.** One device shows a ten-letter code; the other, already
  in the den by its own invite, proves it knows the code by a hash with its
  key, sealed in MLS. Each then says the other is the same person's, and
  members see one person only while both say so. The first device invites
  the second into its private rooms.
- **Key refresh.** Each device replaces its own MLS keys once a day in each
  room (a self-update commit), so a copy of its state taken earlier reads
  nothing after that. The den checks and orders these like any commit.
- **Faster removal.** The door signs tomorrow's tokens only from 22:00 UTC
  (`DEN_TOMORROW_FROM`); devices fetch them at a random moment in those
  hours.
- **Bigger files.** A file is sealed piece by piece, each piece's number and
  whether it is the last authenticated, so none can be reordered or dropped,
  and opened piece by piece as it arrives, with progress either way.
- **Voice for more.** Each browser marks in its packets' headers how loud
  each frame is (RFC 6464); the den ranks speakers by it every 300 ms and
  moves the eight most recent into each listener's slots, rewriting packet
  numbers so a slot that changes speaker stays one stream to the browser.
- **Voice through the den.** Where WebRTC finds no path within eight
  seconds, the page encodes the microphone itself (WebCodecs Opus), seals
  each frame exactly as before and sends it over the den's WebSocket; the den
  forwards relayed and WebRTC frames through one forwarder, so both kinds of
  device hear each other.
- **Big rooms.** A room's GroupInfo is about 190 KB at a thousand members, so
  the den now takes GroupInfos up to 768 KB and commits up to 128 KB, and a
  device writes its MLS state (several MB in a big room) at most once a
  second after reading, at once after anything it sends.

What it costs, measured (2 shared vCPUs, 4 GB, the den in Germany, the load
from a laptop elsewhere): one room of 40 writing members and 400 listening
devices took 50 messages a second with every one delivered, p95 50 ms; the
den topped out near 80 a second (32,000 deliveries a second) at about 500 MB.
In a room of 1000, a device applies a change in 18 ms and the den checks one
in 20 ms. `web/tests/load.mjs` and `web/tests/bench.mjs` reproduce these.

The tests grew with it: the server's go to 58 (one with eleven live WebRTC
devices and one with a relayed device), the browser test to 79 checks (a
relayed member, a member whose WebRTC finds no path, two linked devices, a
30 MB file).

Still open: an independent audit; history for a device that joins later
(MLS gives it none, by design, and a linked device could be sent it); push
notifications while the page is closed.

## 13 October 2026: public

The repository is public, under MIT OR Apache-2.0. The image,
`ghcr.io/zyn-io/epeaky`, is built for x86-64 and arm64, on native
runners for each, and every build is started, checked and installed before
it is pushed. The installer is at
[epeaky.zyn-network.com](https://epeaky.zyn-network.com):

```sh
curl -fsSL https://epeaky.zyn-network.com/install.sh | sudo bash
```

It is alpha. It has had one review and no independent audit;
[THREAT_MODEL.md](THREAT_MODEL.md) says what that means. Security reports go
through the repository's private advisories ([SECURITY.md](SECURITY.md)).

Not done yet, that day: voice for members behind strict networks, devices
refreshing their own keys in MLS (post-compromise security), and an
independent audit.

## 13 October 2026: on a real server

We rented a throwaway server (2 vCPU, Ubuntu 24.04, no setup) and installed
a den on it the way a host would:

- The installer installed Docker, wrote `/opt/epeaky` and started the den
  and Caddy. On the second run it took 13 seconds and printed the first
  invite. Caddy had a Let's Encrypt certificate a few seconds later.
- From outside, only TCP 80 and 443 answered. The den, epmd and the Erlang
  distribution port listened on loopback only.
- Two browsers over the internet entered by invite links, wrote to each
  other, sent an encrypted image and talked in a voice room, each hearing
  the other.
- With approval open, a request from outside showed up in `epeaky
  requests`, `epeaky approve` let it in and `epeaky members` counted it.
- The den's log had no device keys or private room ids in it.

It found three bugs:

- **An mDNS listener on every interface.** The WebRTC library starts a
  resolver for `.local` ICE candidates that listens on UDP 5353 on all
  addresses and caches answers from anyone, so the internet could teach the
  den where a `.local` name points. The den now stops it at boot and refuses
  `.local` candidates; nothing a browser needs is lost, since the page
  offers real addresses. The install test in CI now fails if anything of
  the den listens beyond loopback.
- **`epeaky invites` crashed** on dates SQLite returned as text. Every host
  command now has a test.
- **An image that could not be decoded** failed without saying which. It now
  names the file.

Rebuilding the image on the server and restarting the den on the same data
also tried the update path: nothing was lost, and the browser test passed
again.

## 13 October 2026: the review

Before going public we had Epeaky reviewed in four parts (cryptography,
server, page, release), with proof tests for each finding. Nine findings had
to be fixed before anyone else ran it. In plain words:

1. **The Erlang distribution listened on every interface**, and a public
   image would have shipped one shared cookie: anyone could have run code
   on a den without a firewall. It now listens on loopback only, and each
   den makes its own cookie on first start.
2. **Any member could freeze a room** with a commit the den accepts but the
   others cannot apply (the den checks signatures, not the secrets inside a
   commit). Devices now report such a commit, signed, instead of skipping
   it; when most of the members connected to the room report it, the den
   undoes it and bars its sender from changing that room for a day.
3. **A removed member could write as the newcomer who took their seat.**
   Authors are now named from the room as it was at the message's own epoch,
   and a member who has left is refused.
4. **Private rooms were open to anyone who knew their id.** Their channels
   and voice now let in only the room's members.
5. **Invitations could fill the disk.** They are now charged by size and
   number of recipients, and delivered only to known devices.
6. **Anyone could close the approval way in** by filling the queue. The door
   is now limited per address (an IPv6 /64 counts as one), and the host can
   decline every waiting request at once.
7. **Names were 40 bits**, short enough to grind a lookalike. They are now 80
   bits (`m-` and 20 hex digits), marks come from the whole key, and the
   members list warns when two names start alike.
8. **A test named real server addresses.** It uses documentation ranges, and
   the publish script now refuses any address it knows.
9. **The den could give each device its own day key** and so tell members
   apart. Each device pins every day's key the first time it sees it, and
   members tell each other, inside MLS, which key they were shown.

The review's smaller findings are fixed too:
uploads streamed to disk and charged up front, events throttled per
connection, nothing identifying in logs, the session token out of the URL,
one voice seat per device, ICE candidates filtered and capped, idle private
rooms swept, voice frames stamped with their send time and refused after a
minute, the issuer given only a hash of the pass, invitations that wait for
the member's choice, Leave and Mute, images loaded lazily with a pixel cap,
Forget this device, keyboard access to every message action, generated
third-party licenses, backup docs and a stricter CI.

The threat model was rewritten to match. It now says plainly what the
review showed it had overstated: the operator holds the day keys and can let
its own devices in; door and den share an origin, so an operator who
correlates can link a pass to a device by address or timing; a removed pass
loses entry by the end of the next day, not at once; a majority of members,
by lying, can undo an honest member's change.

## 13 October 2026: one command

A den used to take an afternoon: build the wasm, build the page, generate day
keys, publish them, configure a proxy, make the first member. Now:

- one Dockerfile builds the MLS library, the page and the server into one
  image;
- the den makes and publishes its own day keys (45 days ahead, 31 published
  at `/door/keys`);
- `install.sh` sets up Docker, Caddy and `/opt/epeaky`, opens the firewall
  and prints the first invite;
- `epeaky` on the server makes invites, approves requests and updates the
  den.

## 13 October 2026: gates the host chooses

Epeaky began as the private chat of one community, with its entry check
built in. To make it anyone's, the check became a behaviour, `Den.Gate`,
and the den got its own member list with three ways in, chosen by the host:

- **invite**: one-use links that expire, the code after `#` so no server
  logs it;
- **approval**: someone asks with a short note, the host approves or
  declines, and the note is forgotten;
- **external**: the host's own site checks whatever it wants (a wallet, a
  sign-in) and registers a pass by its hash.

A host who wants something else entirely writes a gate module and compiles
it in from `server/extra`. The community's own gate became such an add-on.

The page also moved onto the den's own origin, served by the den under a
policy that runs only its own scripts, with the host's sign-in (if any) in a
separate window, so wallet or login scripts never run next to the keys.

## 11 October 2026: the first den

The first version: an Elixir/Phoenix server and a browser client whose rooms
are MLS groups (RFC 9420, OpenMLS compiled to WebAssembly).

- **Entry** is a blind token (RFC 9474): the door checks a pass and signs a
  token it never sees, and the den admits the device that token names. The
  den's records never put a pass next to a device.
- **Rooms** carry messages, replies, reactions, edits, deletions, mentions
  and typing as MLS application messages. The server orders commits and
  relays ciphertext.
- **Files** are encrypted in the browser with a fresh key each and stored as
  opaque blobs.
- **Voice rooms** forward audio through the den (ex_webrtc), each frame
  encrypted in the browser under a key from the room's MLS group, so the den
  forwards what it cannot hear.

Within two days the server also learned to check every commit against each
room's public MLS state (a Rust NIF around OpenMLS's PublicGroup) before
ordering it, and devices to prove they hold the key they enter with.
