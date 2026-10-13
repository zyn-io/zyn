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
  opens. Wallets and sign-ins run there, not beside the keys.
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

**Leaving.** Removing a pass stops new tokens, but a device keeps tomorrow's
token: entry ends by the end of the next day, not at once.

## Rooms

- Messages, edits, reactions, replies and typing signals are MLS application
  messages, readable only by the room's members. A message's author is the
  member whose leaf signed it, named from the tree at the message's own epoch:
  a late message from someone the room has since taken out is refused, and
  is never credited to whoever took that seat. A display name is derived from
  the author's key (`m-` and 20 hex digits, 80 bits), never chosen; the
  members list warns when two names start alike.
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
- Any member can remove any other member. In open rooms the den tells
  devices whose day has ended (`expired`) and who is present, and members'
  devices take its word, so a den could get them to remove someone from an
  open room (who can join it again). In private rooms devices never remove
  anyone on the den's word.
- A Welcome to a private room can come from any member, under any title.
- Forward secrecy: a device keeps what it has read; nobody, the den
  included, can decrypt a room's past messages later. There is no
  post-compromise security yet: devices do not refresh their own keys, so an
  attacker who once copied a device's state can keep reading its rooms until
  the device leaves them.

## Files

Files and images are encrypted in the browser with AES-GCM under a fresh key
per file, and the key travels inside an MLS message. The den stores an opaque
blob (at most 25 MB) and sees its size, when it was sent, and which devices
download it.

## Voice

Every room has a voice room. The den forwards audio (one stream up, up to
eight down, nine people at most) and cannot hear it. Each frame is encrypted
in the browser with AES-GCM under its sender's own key, derived from the
room's MLS group and renewed every epoch; a frame carries a counter, and a
listener refuses one it has already played. A slot plays only under the key
of the member the den names for it, so a den that relabels who is speaking
is caught. A member the room takes out is heard for at most 5 seconds more.
Voice is refused in a browser that cannot encrypt frames.

The den sees who is in a voice room, when they talk and how much. A frame's
counter is its send time, authenticated, and a listener refuses frames more
than a minute old, so the den can replay what a member said to a later
listener only within that minute (devices whose clocks are more than a minute
apart cannot hear each other). A slot is played only for a member of the room
as this device's own group has it. Any member can derive every other member's voice key, because
frames are not signed, so one member can impersonate another member's voice.
Outsiders cannot.

## What the operator can see

- device keys and which rooms each one is in, private rooms included, and
  every change to a room's membership (which keys were added or removed, and
  by whom), because commits are signed plaintext;
- when each device writes, reacts, types or talks, and how much;
- file sizes, and who downloads which file; the names of open rooms; the room
  each voice frame belongs to;
- network addresses at the door and the den;
- at the door: each pass code, every day, and when it asked for tokens.

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
