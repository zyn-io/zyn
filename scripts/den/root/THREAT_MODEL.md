# What an Epeaky den sees

A den is an Epeaky server for a closed circle: members enter with a pass,
talk in rooms, share files and join voice. It relays and stores ciphertext,
checks membership changes, and never holds a key that decrypts anything.
This page says what each party can and cannot see and do, so a circle can
decide whether that is enough for them. It covers the code as it is; where
something is a limit rather than a guarantee, it says so.

## The parts

- **The client** (the page the den serves, in the browser): holds every key.
  Rooms are MLS groups (RFC 9420; OpenMLS, suite X25519 / AES-128-GCM /
  SHA-256 / Ed25519), built to WebAssembly from `mls/`. The page's policy
  runs only its own scripts: no inline script and none injected into it.
  Browser extensions with access to the site can still read the page and its
  storage (IndexedDB), where the keys are: use a browser profile without
  extensions for a den that matters.
- **The issuer** (on the host's own site, `external` only): checks whatever
  the host requires and hands the page a pass code through a window the page
  opens. Wallets and sign-ins run there, not beside the keys. That window
  needs a way back to the page to hand it the code, so while it is open it
  could also send the den's tab to another address (which would then not
  have the keys, but could look like the den): it is the host's own site,
  trusted as far as the host is.
- **The door** (`/door/*`): checks a member pass and signs a blinded entry
  token (RFC 9474, RSABSSA-SHA384-PSS-Randomized).
- **The den** (`/socket`, `/blob`): admits a device for the day its token
  names, orders each room's commits and relays messages, files and voice.

The door and the den are one server run by one operator, the host.

## Entry

1. A device gets in once, by a way the host opened: an invite link, the
   host's approval, or the host's own site registering a pass (`external`).
   The device makes its pass code itself and the den keeps only its hash; in
   `external` the host's issuer is given only that hash, unless the host
   chose to have its issuer make the code (`issuerCode`), in which case the
   issuer knows it. Either way the host knows whom it invited, approved or
   registered, and which pass that was.
2. Each day the device asks the door for a token, blinded, and shows its pass
   code: the door sees the code itself every day. It checks the pass, signs
   at most 3 tokens per pass per day, and never sees a token.
3. The client unblinds the token and presents it to the den with its device
   key and an Ed25519 signature by that key over `den-enter-v1|<token>`. The
   den checks the day's signature and the proof of key, and spends the token
   once.
4. The door signs with one key per day, which the den publishes ahead
   (`/door/keys`). A device pins each day's key the first time it sees it and
   refuses a different one later; and in every room each device tells the
   others, sealed, which key it was shown for the day. A den that gives one
   device keys of its own from its first visit is seen by the other members
   (by name: the member reporting it may also be lying). A device alone in
   all its rooms would not notice.

**What blind entry gives you.** The den's records never put a pass next to a
device: the database holds no pass, and a token is not stored with the pass
it was signed for.

**What it does not give you.** The door and the den run on one server, on one
origin, and see the same network address. An operator who looks can link a
pass to a device by address, or by timing (a device that has no token kept
signs and enters within moments), and that link holds for as long as the
device key does. The page cannot reach the door and the den by different
paths. Blind tokens are no protection against an operator who correlates.

**The operator can always let itself in.** It holds the door's keys, so it
can sign tokens for devices of its own, which appear as new, anonymous
members of open rooms.

**Leaving.** Removing a pass stops new tokens, but a device may hold
tomorrow's token, which the door signs only from 22:00 UTC (the host's
`DEN_TOMORROW_FROM`): a pass removed before then admits nobody after midnight
(an hour's grace); one removed later, nobody after the next midnight. Not at
once in either case.

## Rooms

- Messages, edits, reactions and replies are MLS application messages,
  readable only by the room's members. A message's author is the
  member whose leaf signed it, named from the tree at the message's own epoch:
  a late message from someone the room has since taken out is refused, and
  is never credited to whoever took that seat. A device's name is derived
  from its key (`m-` and 20 hex digits, 80 bits), never chosen; the members
  list warns when two names start alike.
- Typing signals are not sealed: a device tells the den it is typing, and
  the den tells the room which device. So the den sees when someone types
  (as it sees when they write), and can show anyone as typing. They carry no
  words.
- A member may also give themselves a name. It travels sealed (the den never
  sees it) and is a claim: anyone can choose any name, so the page shows it
  beside the start of the device's own name, and warns when two members of a
  room chose the same one.
- A member's devices can be linked: one shows a code, the other proves it
  knows the code inside MLS (a hash with its key; the code itself never
  travels), and each then says, sealed, that the other is the same
  person's. Devices count as one person only while each says so of the
  other; a device can claim another, but not be claimed into a link it never
  agreed to.
- Commits (who is in a room) are sent as MLS PublicMessages: signed, not
  encrypted. The den keeps each room's public state (OpenMLS's PublicGroup,
  in `server/native/`) and orders a commit only if it verifies against that
  state and its signer is the device whose session sent it. Open rooms take
  no Adds: each device joins one by its own external commit.
- The den cannot check a commit's secrets (its membership tag, the path
  secrets it encrypts to each member). A member can therefore send a commit
  the den accepts and others cannot apply. A device never skips such a
  commit quietly: it reports it, signed, and says so on screen. When most of
  the members connected to the room report the same commit, the den undoes
  it, and its sender may not change that room for a day. An open room's
  commit whose GroupInfo never comes is undone after 30 seconds, and a
  device left behind joins the room again. What remains: a member can disrupt
  a room until the reports come in; a member who is the only one unable to
  apply a commit in a private room has to be invited again; and a majority of
  members, by lying, can undo an honest member's change and bar them for a
  day.
- A room's GroupInfo, which new members join from, is accepted only if it
  matches the den's state for that room.
- Proposals sent on their own are ignored; changes travel only as commits.
- Private rooms have a random 128-bit id that the den never lists. Their
  channels and voice let in only the room's members (or its creator before
  its group exists), and a commit that takes a device out closes its
  channels at once. They are joined by invitation only: an invitation must
  carry the key that was asked for, a private room refuses external joins
  (at the den and in every device), and a device accepts a Welcome only for a
  private room it is not already in.
- In an open room the den orders a commit that takes a live member out only
  from a device whose roles allow Kick (and are above that member's), or for
  a device whose day ended or that moderators kicked or banned. The den tells
  members about those (`expired`) and who is present, and members' devices
  take its word, so a den could get them to remove someone from an open room
  (who can join it again unless banned). In a private room any member can
  remove any other, and devices never remove anyone on the den's word.
- Moderation is enforced by the den on what it already knows: which device
  sends what, where, when. A device timed out, kicked or banned has its
  messages, reactions, typing and voice refused (its own commits still pass,
  so its state stays in step); a channel's slow mode lets each device send a
  few messages a window, and pages hide a second message or file from one
  device inside the window by the den's own time on each message; "who may
  post" refuses the rest. Moderators delete a message by its number: the den
  drops its ciphertext and tells the room. The den keeps no record of who
  sent which stored message.
- Roles, who holds them, timeouts, kicks and bans are by device key, and
  public to members (as on Discord). Bans cannot follow a person: the den
  cannot link a device to the pass it came in with, by design, so someone
  banned can return on a new device under a new device name (at most three a
  day per pass). A malicious den could invent roles, bans or deletions; it
  could always refuse service. It still cannot read, forge or change a
  message.
- A Welcome to a private room can come from any member, under any title.
- Forward secrecy: a device keeps what it has read; nobody, the den
  included, can decrypt a room's past messages later.
- Post-compromise security: every device replaces its own keys in each room
  once a day (an MLS self-update commit, at a random moment). Someone who
  once copied a device's state can read that device's rooms until its next
  refresh, then nothing more. This holds against a copy that only reads: the
  copy also holds the device's signing key, so someone who acts with it
  first can take the device's place in a room (the device then cannot apply
  the room's changes and says so).

## Files

Files and images are encrypted in the browser with AES-GCM under a fresh key
per file, in 1 MiB pieces whose number and last-piece mark are
authenticated (none can be reordered or dropped), and the key travels inside
an MLS message. The den stores an opaque blob (at most the host's `file_max`,
100 MB by default) and sees its size, when it was sent, and which devices
download it.

## Voice

Every room has a voice room. The den forwards audio (one stream up, eight
down, up to 50 people) and cannot hear it. Each listener hears the eight who
spoke most recently, which the den tells from the audio level each browser
puts in its packets' headers (RFC 6464), outside the frame's encryption. A
device whose network lets no WebRTC through sends and receives the same
sealed frames over its connection to the den, with the same level beside
each. Each frame is encrypted
in the browser with AES-GCM under its sender's own key, derived from the
room's MLS group and renewed every epoch; a frame carries a counter, and a
listener refuses one it has already played. A slot plays only under the key
of the member the den names for it, so a den that relabels who is speaking
is caught. A member the room takes out is heard for at most 5 seconds more.
Voice is refused in a browser that cannot encrypt frames.

The den sees who is in a voice room, when they talk and how much, and how
loud each frame is (in steps of a decibel, every 20 ms). Loudness and frame
sizes over time can give away something of what is said, as research on
encrypted voice has shown: voice is less private than written words. A frame's
counter is its send time, authenticated, and a listener refuses frames more
than a minute old, so the den can replay what a member said to a later
listener only within that minute (devices whose clocks are more than a minute
apart cannot hear each other). A slot is played only for a member of the room
as this device's own group has it. Any member can derive every other member's voice key, because
frames are not signed, so one member can impersonate another member's voice.
Outsiders cannot.

### Screen sharing

One member of a voice room at a time may share their screen; the others
watch it on a stage. The presenter's browser encrypts each VP8 frame with
AES-GCM under the presenter's own screen key (the MLS exporter labelled
`den screen` with their key, renewed every epoch, distinct from their voice
key), with the same counter, replay window and one-minute rule as voice. The
first bytes of each frame stay in the clear because WebRTC needs them: 10 on
a keyframe, 3 otherwise. They are covered by the tag, so they cannot be
changed. A viewer opens the stage only under the key of the member the den
says presents, and only for a member of the room as its own group has it. As
with voice, any member can derive a presenter's screen key, so a member
working with the den could show frames as someone else's screen.

The den sees who presents, when and for how long; how much video they send
and when keyframes come; and, from the bytes left in the clear, the size of
the shared picture and its scaling (keyframes), and for every frame the size
of its first partition and whether it is shown. Frame sizes over time say
something about how much on the screen changes. Not what is on the screen. A
member whose voice goes over the den's own connection can neither share nor
watch.

## Agents

An agent (`agent/`, `epeaky-agent`) is a device like any other: the den's
own client in Node, its key, pass and state in a folder on the machine it
runs on. The den sees what it sees of any device, and nothing that tells it
the device is an agent: that, and whose agent it is, are said inside MLS.

- An agent takes orders only from its owner. Both sides say so in sealed
  profiles: the agent names the owner's device, the owner's device names the
  agent (after a code shown on the owner's page reaches the agent, checked by
  hash as in device linking). Devices linked to the owner count only when
  both sides say they are linked. The den cannot forge any of it.
- Every message the agent reads carries its sender's kind (owner, agent,
  member, the den), taken from the sender's key, on one line. What the agent
  does with words that are not its owner's is up to the model behind it; the
  marking is what lets it tell them apart.
- Anyone who can read the agent's folder holds its key and pass, as with a
  browser profile.
- "Add an agent" on the page of a member who may make invites makes a
  single-use invite (a day) and puts it in the command it shows, with the
  code. The command is a key: whoever runs it first joins and becomes that
  member's agent, while the page is open and within ten minutes.
- On Windows, the agent's folder is protected by the ACL of where it lies:
  in the user's profile (the default), only that user. Commands check that
  the background process proves it holds the agent's secret, so a program of
  another user's listening on its pipe cannot pass for it.

- **An owner's rules for an agent** (which rooms it acts in, which of the
  owner's other agents' tasks it takes as orders, files) are sealed
  documents from the owner, and the agent keeps to them itself: the den
  cannot see into rooms to hold it to them. They stop the model behind an
  agent doing more than its owner allows; they do not hold back a machine
  someone else controls. Each room is told only whether it is one of the
  agent's rooms. A delegated order is only ever a task addressed to the
  agent; another agent's ordinary words stay information.
- **Who told an agent what** is the agent's own account. What it sends says
  what it was for (a message of the same room: the order it replies to, the
  task it reports on, or one it names), sealed like its words; the pages
  call it an order only when it points to the agent's owner's words. Tasks
  keep their history on every page. The agent's journal (`epeaky-agent log`)
  holds the words of every order it was handed, private rooms' included, in
  plaintext in its folder, protected as its keys are. None of it is proof: an
  agent, or whoever controls its machine, can name the wrong cause or keep a
  false journal. What the room shows the agent's key sent is what it did. The
  den's moderation audit log is separate: what moderators did, never what
  anyone wrote; the den cannot see any of this.

## Pins

A pin is a sealed document from the member who pinned it: the words, and
which message they came from as the pinner says. Every member's device sends
its own pins again when it sees someone join, so a newcomer gets each pin
from its pinner's key, never passed on by someone else. A member who may
manage messages can unpin anyone's pin in an open room; the pinner's device
then stops sending it. A pin is known by its pinner and its id, so nobody's
pin can stand in for another's. A pin's words are the pinner's claim: an
agent reads a member's pin of their own words with that member's kind, and
a pin of someone else's message as information, whoever pinned it.

A device reads only what was sent after it joined, and search runs only over
what it holds; two members, or two agents, can hold different histories.

## Pushes

A device that turns on notifications gives the den its browser's push address
(Den.Push). When words land in a room it is in and it is not connected, the
den sends a push through the browser's push service: encrypted for that
browser (RFC 8291) and signed with the den's own key (RFC 8292), saying the
den's name and the room's ("a private room" for one), never the words or who
wrote them. The den sends only to Apple's, Google's, Mozilla's and
Microsoft's push addresses, never follows a redirect, and sends at most once
a device and room every two minutes, so a member cannot make it call an
address of their own.

The push service learns more than the words it cannot read: when a push
reaches a device; which den sent it (the den's address and its one signing
key, the same for all its members); and, because one message wakes a room's
members who are away at about the same time, which of its users are likely
in a room together. Each push waits a random few seconds to blur that, which
helps against a casual look, not against a service that sets out to map a
den. With pushes for private rooms only, those are small rooms: their
members are the easier to tell. A member who does not want this keeps
notifications to while the page is open.

## What the operator can see

- device keys and which rooms each one is in, private rooms included, and
  every change to a room's membership (which keys were added or removed, and
  by whom), because commits are signed plaintext;
- when each device writes, reacts, types or talks, and how much (in voice,
  how loud, frame by frame);
- file sizes, and who downloads which file; the names of open rooms; the room
  each voice frame belongs to;
- who shares their screen in a voice room, when and for how long, how much
  video they send, and the size of the shared picture;
- network addresses at the door and the den;
- at the door: each pass code, every day, and when it asked for tokens;
- roles and who holds them, timeouts, kicks and bans, and the audit log of
  what moderators did (90 days; never what anyone wrote).
- for a device with notifications on: its browser's push address, whether it
  wants pushes for every room or private ones only, and which of its
  documents are words (a page marks those, so they alone send pushes);
  whose pushes are sent when.

## What nobody but members can see

The words of messages, file contents and names, voice audio, and which
member holds which pass (unless the operator correlates, as described under
Entry).

## Retention

Ciphertext and blobs are kept 30 days, then deleted (SQLite overwrites what
it deletes). A device whose day ended without a new token is removed from its
rooms by the members' own devices. Backups, if the host keeps any, should
follow the same 30 days.

## Not covered

- A malicious page. The den serves the page that holds the keys, so whoever
  runs the den can change it and take them. Serve nothing else from the den's
  origin, and check the page you are given if you cannot trust the host.
- A compromised member device, or a member who shares what they read.
- Traffic analysis beyond what is listed above.

The den has had one review (cryptography, server, page and release, October
2026), whose findings this version addresses; it has not had an independent
security audit.
