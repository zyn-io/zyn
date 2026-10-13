# What the den sees

The den is a chat server for a closed community: members enter with a pass,
talk in rooms, share files and join voice. It relays and stores ciphertext,
checks membership changes, and never holds a key that decrypts anything. This page says what each party can and cannot see, so a
community can decide whether that is enough for them.

## The parts

- **The client** (in the browser): holds every key. Rooms are MLS groups
  (RFC 9420; OpenMLS, suite X25519 / AES-128-GCM / SHA-256 / Ed25519), built
  to WebAssembly from `mls/`.
- **The door** (`/door/*`): checks a member pass and signs a blinded entry
  token (RFC 9474, RSABSSA-SHA384-PSS-Randomized).
- **The den** (`/socket`, `/blob`): admits a device for the day its token
  names, orders each room's commits and relays messages, files and voice.

The door and the den are one server run by one operator.

## Entry

1. The client asks the door for a token for today or tomorrow, blinded, with
   the member's pass. The door checks the pass (the Zyn adapter asks a Zyn node
   whether the pass is live), signs at most 3 tokens per pass per day, and
   never sees the token itself.
2. The client unblinds the token and presents it to the den together with its
   device key and an Ed25519 signature by that key over
   `cave-den-enter-v1|<token>`. The den checks the day's signature and the
   proof of key, then spends the token once.
3. Each day has one signing key, published ahead in one file for everyone. A
   client takes keys only from that file, so the door cannot give each member
   a key of their own to recognise them by.

**What blind entry gives you.** The den's records never put a pass next to a
device: the database holds no pass, and a token is not stored with the pass it
was signed for.

**What it does not give you.** The door and the den run on the same server
and see the same network address. An operator who looks can link a pass to a
device by address, or by one timing match on the member's first day, and that
link then holds for as long as the device key does. Blind tokens are no
protection against an operator who correlates. A member who needs that must
reach the door and the den over different network paths, for example
through a VPN or Tor for one of them.

## Rooms

- Messages, edits, reactions, replies and typing signals are MLS
  application messages, readable only by the room's members. A message's
  author is its MLS sender's signature key; a display name is derived from that
  key (`m-` and 10 hex characters), never chosen.
- Commits (who is in a room) are sent as MLS PublicMessages: signed, not
  encrypted. The den keeps each room's public MLS state (OpenMLS's
  PublicGroup, in `server/native/`) and accepts a commit only if it verifies
  against that state and its signer is the device whose session sent it. A
  member cannot freeze a room with a commit nobody can process, and nothing is
  ever rolled back.
- A room's GroupInfo, which new members join from, is accepted only if it
  matches the den's state for that room (the same group context and a valid
  signature), so nobody can send newcomers into a forked copy of a room.
- Proposals sent on their own are ignored; changes to a room travel only as
  commits.
- Private rooms have a random 128-bit id that the den never lists, and are
  joined by invitation only: an invitation must carry the key that was asked
  for, a private room refuses external joins, and a device accepts a Welcome
  only for a private room it is not already in.
- Forward secrecy: a device keeps what it has read. Nobody, the den
  included, can decrypt a room's past messages later.

## Files

Files and images are encrypted in the browser with AES-GCM under a fresh key
per file, and the key travels inside an MLS message. The den stores an opaque
blob (at most 25 MB) and sees its size and when it was sent.

## Voice

Every room has a voice room. The den forwards audio (one stream up, up to
eight down, nine people at most) and cannot hear it. Each frame is encrypted
in the browser with AES-GCM under its sender's own key, derived from the
room's MLS group and renewed every epoch, and carries an authenticated counter
so a replayed frame is refused. A slot plays only under the key of the member
the den names for it, so a den that relabels who is speaking is caught. Voice
is refused in a browser that cannot encrypt frames.

The den sees who is in a voice room, when they talk and how much. Any member
can derive every other member's voice key, because frames are not signed, so
one member can impersonate another member's voice. Outsiders and the den
cannot.

## What the operator can see

- device keys and which rooms each one is in, private rooms included, and
  every change to a room's membership (which keys were added or removed, and
  by whom), because commits are signed plaintext;
- when each device writes, reacts, types or talks, and how much;
- file sizes; the names of open rooms; the room each voice frame belongs to;
- network addresses at the door and the den;
- at the door: which passes asked for tokens, and when.

## What nobody but members can see

The words of messages, file contents and names, voice audio, and which member
holds which pass (unless the operator correlates, as described under Entry).

## Retention

Ciphertext and blobs are kept 30 days, then deleted. A device whose day
ended without a new token is removed from its rooms by the members' own
devices, so a member who loses their pass loses entry at the end of that day,
not within minutes. Backups, if the operator keeps any, should follow the same
30 days.

## Not covered

- A malicious client page. The page holds the keys, so whoever serves it can
  take them. Serve it from a domain that hosts nothing else.
- A compromised member device, or a member who shares what they read.
- Traffic analysis beyond what is listed above.

The den has not had an independent security audit.
