# Devlog

Newest first. What changed, why, and what we found along the way.

## 13 October 2026: public

The repository and the image (`ghcr.io/zyn-io/epeaky`) are public, under
MIT OR Apache-2.0. The image is built for x86-64 and arm64, on native
runners for each, and every build is started, checked and installed before
it is pushed. The installer is at
[epeaky.zyn-network.com](https://epeaky.zyn-network.com):

```sh
curl -fsSL https://epeaky.zyn-network.com/install.sh | sudo bash
```

It is alpha. It has had one review and no independent audit;
[THREAT_MODEL.md](THREAT_MODEL.md) says what that means. Security reports go
through the repository's private advisories ([SECURITY.md](SECURITY.md)).

Not done yet: a TURN server option for members behind strict NATs, devices
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
