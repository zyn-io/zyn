# Screen sharing in voice rooms

16 October 2026. Agreed in chat: the first step towards video. Someone in a
voice room shares their screen and everyone else in that voice room watches.
One presenter at a time, anyone in the voice room may present (first come),
and a moderator who may kick can stop a presentation. End-to-end encrypted
like voice. Cameras, sound from the shared screen and several presenters at
once are later steps.

## What exists

- `Den.Voice` (server/lib/den/voice.ex): one GenServer per voice room, an SFU
  on ex_webrtc 0.17. Each device has one `PeerConnection`: its first audio
  transceiver sends to the den, the next eight are slots down. The den
  forwards each speaker's packets into listeners' slots, continuing sequence
  numbers and timestamps when a slot changes speaker. `audio_codecs: [:opus],
  video_codecs: []`.
- `DenWeb.VoiceChannel`: signalling (`offer`, `ice`, `relay_join`, `frame`,
  `leave`); a timed-out device cannot use it; a device taken out of a private
  room leaves voice.
- The page (`web/den.js`, `class Voice`): frames are sealed in a worker
  (`web/voice-worker.js`) with AES-GCM under the sender's key for the epoch,
  from the MLS exporter labelled `den voice` and the sender's key, through
  `RTCRtpScriptTransform` (Safari, Firefox) or `createEncodedStreams`
  (Chrome). A counter (send time in µs) refuses replays and frames over a
  minute old.
- Relayed voice: a device whose network lets no WebRTC through sends and gets
  sealed audio frames over the voice channel.
- ex_webrtc 0.17 has what video needs: VP8 (`ExWebRTC.RTP.VP8.keyframe?/1`),
  `ExWebRTC.RTP.Munger` (one continuous stream from changing sources, VP8
  picture ids included), `PeerConnection.send_pli/3`, and on by default NACK
  and retransmission both ways (`:inbound_rtx`, `:outbound_rtx`) and TWCC.

## What a member sees

- In voice, on a desktop browser with a direct (WebRTC) connection, a
  **Share screen** button. The browser asks which screen, window or tab.
- While someone presents, the voice room's page shows a stage: their screen,
  fitted to the space, with "*name* is sharing their screen" and **Full
  screen**. In another room, the voice bar says who is sharing and **Watch**
  goes to the voice room.
- The presenter sees "You are sharing your screen" and **Stop sharing**. The
  browser's own stop button does the same.
- If someone already presents, **Share screen** is disabled with "*name* is
  sharing" as its reason.
- A moderator who may kick sees **Stop** on the stage. The presenter's page
  stops capturing and says "*moderator* stopped your screen share". It goes
  in the audit log (`screen_stop`, the presenter's key).
- On a phone (no `getDisplayMedia`), there is no **Share screen**, but
  watching works.
- A relayed member (no WebRTC) can neither present nor watch. The stage says
  "Screen sharing needs a direct connection. This network only lets voice
  through."

## How it works

### Connections

At join, the page's offer has, after today's nine audio transceivers, two
video transceivers: one `sendonly` with no track (the screen, if this device
presents) and one `recvonly` (the stage). Starting to present only puts a
track on the first (`replaceTrack`), so there is no renegotiation and nothing
to set up when the presenter changes.

The den answers with `video_codecs` VP8 only (90 kHz), takes the first video
transceiver as the device's video up (`recvonly` on the den's side) and the
second as its stage (`sendonly`, one `MediaStreamTrack.new(:video)`). An
offer with no video transceivers (a page from before this change) joins
voice as today, without screen sharing; its peer has `screen: nil`.

### Presenting

1. The page calls `getDisplayMedia({video: {frameRate: 15}, audio: false})`,
   sets `contentHint = 'detail'`, then sends `present` on the voice channel.
2. The room's GenServer accepts if nobody presents, the device is `:rtc` with
   a video up, and it is not timed out: `presenter = channel`. It tells every
   peer `screen: {key}`, asks the presenter for a keyframe, and replies `ok`.
   Otherwise it replies `{error: busy, key}` (or `unsupported`), and the page
   stops the capture.
3. On `ok`, the page puts the track on its video sender with
   `maxBitrate` = the den's `screen_bitrate` and `maxFramerate` 15
   (`setParameters`). The voice join reply carries `screen_bitrate`.
4. `unpresent`, the track ending, leaving voice, the connection failing, a
   timeout, or a moderator's `stop_screen` frees the stage: `presenter = nil`,
   and every peer is told `screen: null`.

`stop_screen` is accepted only from a device in that voice room for which
`Den.Mod.can?(key, "kick")`; the den logs it and tells the presenter
`screen_stopped: {by}`.

### Forwarding

- RTP from the presenter's video up track goes through one `Munger` (VP8,
  90 kHz) per voice room, then to the stage track of every other `:rtc` peer
  that has one. On a new presenter, `Munger.update/1`, so viewers see one
  continuous stream.
- A viewer gets packets only from the first keyframe after it joined or the
  presenter changed (`waiting_key` per viewer), so it never decodes half a
  picture.
- Keyframes: the den sends a PLI to the presenter when a presenter starts,
  when a viewer starts waiting, and when a viewer sends a PLI or FIR. At most
  one a second per voice room; the rest wait for the next. Lost packets are
  resent by ex_webrtc on each leg.
- Video from anyone who is not the presenter is dropped.
- Bandwidth guard: the den counts the presenter's video bytes. Above twice
  `screen_bitrate` averaged over 5 seconds, it stops the presentation and
  tells the presenter `screen_stopped: {by: null, reason: "too fast"}`. This
  stops one device from multiplying its upload by every viewer.

### Encryption

- Each presenter's frames are sealed with AES-GCM under their own key for the
  epoch, from the MLS exporter labelled `den screen` and the sender's key. It
  is a different key from their voice key.
- The worker leaves the start of each VP8 frame in the clear, as WebRTC needs
  it: 10 bytes on a keyframe, 3 otherwise (bit 0 of byte 0 is 0 on a
  keyframe). These bytes are authenticated (AES-GCM additional data), so
  changing them fails to open. The rest of the frame and the tag follow, with
  the same tail as voice frames: iv, counter, key index.
- The counter, replay window and one-minute staleness rule are those of
  voice. A viewer opens a stage frame only under the key of the device the
  den says presents. A mismatch fails to open and shows nothing.

## Settings

| Setting | Default | |
|---|---|---|
| `DEN_SCREEN_BITRATE` | 2500000 | bits a second a presenter may send; 300000 to 8000000 |
| `DEN_SCREEN` | `on` | `off` turns screen sharing off for the den |

Upload from the den is about `screen_bitrate` × viewers: 2.5 Mbps to 49
viewers is about 120 Mbps. The README says so beside the setting.

## What the server sees

THREAT_MODEL.md and the site's "The server sees" list gain:

- who presents, when, and for how long;
- how much video they send, and when keyframes come;
- the size of the shared picture, which is in the VP8 keyframe header that
  stays in the clear.

Not what is on the screen.

## Out of scope

- Cameras, several videos at once, simulcast.
- Sound from the shared tab or screen.
- Screen sharing for relayed members.
- Recording.

## Tests

- **Server** (`test/den/voice_test.exs` and the channel test):
  - one presenter at a time, and a second `present` gets `busy` with the
    presenter's key;
  - the stage is freed on `unpresent`, on leaving, on a dropped connection
    and on a timeout;
  - `stop_screen` works with kick and is refused without it, and lands in
    the audit log;
  - only the presenter's video is forwarded;
  - a viewer waits for a keyframe;
  - PLIs to the presenter are limited to one a second;
  - the bandwidth guard stops a presenter;
  - an offer without video still joins voice;
  - `DEN_SCREEN=off` refuses `present`.
- **Worker** (page unit tests): a video frame seals and opens; the clear
  prefix is 10 or 3 bytes; a changed clear byte or a wrong sender's key fails
  to open; a replayed counter is refused.
- **Browser** (`web/tests/den.e2e.mjs`, Chrome): `getDisplayMedia` returns
  a canvas stream (an init script; no test hook in the page) whose colour
  changes every second.
  - A viewer's stage shows a picture (`videoWidth` > 0, frames rising) and
    its centre pixel follows the canvas's colour.
  - A second presenter is refused while one presents.
  - **Stop sharing** frees the stage, and a moderator's **Stop** does too.
  - A relayed member sees the "needs a direct connection" note.
- **By hand:** Safari and Firefox as presenter and viewer; a phone as viewer;
  then the test den gen-epeaky.zyn-network.com with three devices.

## Rollout

Core, so public (zyn-io/epeaky) through scripts/den/sync.sh, with README
(setting, bandwidth), THREAT_MODEL, DEVLOG and the site's features and "The
server sees". Then the test den gen-epeaky.zyn-network.com updates with
`epeaky update`, which the user runs, since remote writes there are theirs.
