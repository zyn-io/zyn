# Screen sharing in voice rooms: implementation plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** One member of a voice room shares their screen and everyone else in it
watches, sealed end to end.

**Architecture:** Each voice connection gets two more transceivers, video up
and a video "stage" down. The den's room process forwards only the
presenter's VP8 packets, through one `ExWebRTC.RTP.Munger`, to every viewer
that has seen a keyframe. The page seals screen frames in the voice worker
under a `den screen` key per sender, leaving VP8's first 10 bytes (keyframe)
or 3 bytes (delta) in the clear and authenticated.

**Tech stack:** Elixir and ex_webrtc 0.17 (server/lib/den/voice.ex,
DenWeb.VoiceChannel), browser WebRTC with `RTCRtpScriptTransform` or
`createEncodedStreams`, WebCrypto AES-GCM, node:test, Playwright with
system Chrome.

**Spec:** apps/cave-den/docs/specs/2026-10-16-screen-share.md

## Global constraints

- One presenter per voice room. Anyone in the voice room may present, first
  come. `stop_screen` only from a device in that voice room with
  `Den.Mod.can?(key, "kick")`, logged as `screen_stop`.
- VP8 only, 90 kHz. Clear prefix: 10 bytes on a keyframe (bit 0 of byte 0 is
  0), 3 otherwise. Authenticated as AES-GCM additional data.
- Key: `this.den.export(room, \`den screen|${sender}\`, 32)`, separate from
  `den voice`.
- `DEN_SCREEN` (`on` | `off`, default on), `DEN_SCREEN_BITRATE` (default
  2_500_000, 300_000..8_000_000; anything else raises at boot).
- Capture: `getDisplayMedia({video: {frameRate: 15}, audio: false})`,
  `contentHint = 'detail'`, `maxFramerate` 15, `maxBitrate` = screen_bitrate.
- At most one PLI a second to the presenter per voice room. The bandwidth
  guard stops a presenter above 2 × screen_bitrate averaged over 5 s.
- Copy is plain, with these labels: "Share screen", "Stop sharing",
  "*name* is sharing their screen", "You are sharing your screen",
  "Full screen", "Watch", "Stop", "*name* is sharing",
  "*name* stopped your screen share",
  "Screen sharing needs a direct connection. This network only lets voice through."
- No test hooks in shipped page code. The e2e replaces `getDisplayMedia`
  from an init script.
- Old pages (no video transceivers in the offer) still join voice.

## Review focus

1. **The presenter's own page:** it must not try to open its own stage
   frames. The den never sends the presenter's video back to them. Test in
   Task 4: the presenter gets no stage packets.
2. **The presenter leaving voice while sharing** (tab closed, network gone)
   must free the stage for everyone within the connection timeout. Test in
   Task 3: drop on `:DOWN`.
3. **Two devices pressing Share screen at once:** exactly one wins, and the
   other's capture is stopped with "*name* is sharing". Test in Task 3
   (server) and Task 8 (e2e).
4. **An epoch change during a presentation** (someone joins the room): the
   viewers' screen keys must refresh like voice keys, or the stage freezes.
   Test in Task 8: a third member joins the room mid-share and the stage
   keeps updating.
5. **A viewer joining mid-share** gets a picture within about a second, with
   no half-decoded frame. Test in Task 4: a late viewer's first packet is a
   keyframe, and the presenter was asked for one.

---

### Task 1: Settings and what the join tells the page

**Files:**
- Modify: `server/config/runtime.exs` (top level, after `DEN_VOICE_PUBLIC_ONLY`'s neighbours)
- Modify: `server/lib/den/voice.ex` (`Den.Voice`: `screen?/0`, `screen_bitrate/0`)
- Modify: `server/lib/den_web/channels/voice_channel.ex` (join reply)
- Test: `server/test/den/voice_test.exs`

**Interfaces:**
- Produces: `Den.Voice.screen?() :: boolean`, `Den.Voice.screen_bitrate() :: pos_integer`; the voice join reply `%{slots: 8, screen: boolean, screen_bitrate: integer}`.

- [ ] **Step 1: Failing test**

```elixir
  test "screen sharing is on by default at 2.5 Mbps, and the host can turn it off" do
    Application.delete_env(:den, :screen)
    Application.delete_env(:den, :screen_bitrate)
    assert Den.Voice.screen?()
    assert Den.Voice.screen_bitrate() == 2_500_000
    Application.put_env(:den, :screen, false)
    refute Den.Voice.screen?()
    Application.delete_env(:den, :screen)
  end
```

- [ ] **Step 2: Run it** with `cd server && mix test test/den/voice_test.exs`. Expected: fails with "undefined function screen?/0".

- [ ] **Step 3: Implement**

In `Den.Voice`:

```elixir
  @doc "Whether members may share their screen in voice (`DEN_SCREEN`, on by default)."
  def screen?, do: Application.get_env(:den, :screen, true)

  @doc "Bits a second a presenter may send (`DEN_SCREEN_BITRATE`, 2.5 Mbps)."
  def screen_bitrate, do: Application.get_env(:den, :screen_bitrate, 2_500_000)
```

In `runtime.exs`, at top level so dev and test read it too:

```elixir
# Screen sharing in voice (Den.Voice.screen?/0, screen_bitrate/0).
case System.get_env("DEN_SCREEN") do
  nil -> :ok
  v when v in ["on", "1", "true"] -> config :den, screen: true
  v when v in ["off", "0", "false"] -> config :den, screen: false
  v -> raise "DEN_SCREEN must be on or off, not #{inspect(v)}"
end

if v = System.get_env("DEN_SCREEN_BITRATE") do
  case Integer.parse(v) do
    {n, ""} when n in 300_000..8_000_000 -> config :den, screen_bitrate: n
    _ -> raise "DEN_SCREEN_BITRATE must be bits a second, 300000 to 8000000"
  end
end
```

In `VoiceChannel.join/3`, the reply:

```elixir
{:ok, %{slots: Den.Voice.slots(), screen: Den.Voice.screen?(), screen_bitrate: Den.Voice.screen_bitrate()}, assign(socket, :voice, room)}
```

- [ ] **Step 4: Run** `mix test test/den/voice_test.exs`. Expected: pass.
- [ ] **Step 5: Commit** "Voice: DEN_SCREEN and DEN_SCREEN_BITRATE, told to the page at join".

### Task 2: Video transceivers in the answer

**Files:**
- Modify: `server/lib/den/voice.ex` (`Den.Voice.Room.answer/4`, `on_pc({:track, …})`, peer map)
- Test: `server/test/den/voice_test.exs`

**Interfaces:**
- Produces: peer fields `screen_up: track_id | nil` (the device's video up, once its track arrives), `stage: track_id | nil` (the den's video track down), `stage_ready: false`. `device(video: true)` test helper.

- [ ] **Step 1: Failing tests**

```elixir
  defp device(opts \\ []) do
    {:ok, pc} = PeerConnection.start(audio_codecs: [:opus], video_codecs: if(opts[:video], do: [:vp8], else: []))
    {:ok, _} = PeerConnection.add_track(pc, MediaStreamTrack.new(:audio))
    [up] = PeerConnection.get_transceivers(pc)
    :ok = PeerConnection.set_transceiver_direction(pc, up.id, :sendonly)
    for _ <- 1..Den.Voice.slots(), do: {:ok, _} = PeerConnection.add_transceiver(pc, :audio, direction: :recvonly)
    if opts[:video] do
      {:ok, _} = PeerConnection.add_track(pc, MediaStreamTrack.new(:video))
      v = PeerConnection.get_transceivers(pc) |> Enum.find(&(&1.kind == :video))
      :ok = PeerConnection.set_transceiver_direction(pc, v.id, :sendonly)
      {:ok, _} = PeerConnection.add_transceiver(pc, :video, direction: :recvonly)
    end
    {:ok, offer} = PeerConnection.create_offer(pc)
    :ok = PeerConnection.set_local_description(pc, offer)
    {pc, SessionDescription.to_json(offer)}
  end

  defp media(sdp), do: Regex.scan(~r/m=(audio|video) (\d+)[^\r\n]*\r\n(?:(?!m=).*\r\n)*?a=(sendonly|recvonly|sendrecv|inactive)/, sdp, capture: :all_but_first)

  test "an offer with a screen and a stage is answered with VP8: the den takes the screen and sends the stage" do
    {:ok, room} = Den.Voice.ensure("screen-answer")
    {pc, offer} = device(video: true)
    {:ok, answer} = Den.Voice.Room.join(room, self(), "key-a", offer)
    videos = for [k, port, dir] <- media(answer["sdp"]), k == "video", do: {port, dir}
    assert [{p1, "recvonly"}, {p2, "sendonly"}] = videos
    assert p1 != "0" and p2 != "0"
    assert answer["sdp"] =~ "VP8/90000"
    refute answer["sdp"] =~ ~r/H264|AV1/
    PeerConnection.stop(pc)
  end

  test "an offer without video (a page from before screens) still joins voice" do
    {:ok, room} = Den.Voice.ensure("screen-old")
    {pc, offer} = device()
    assert {:ok, answer} = Den.Voice.Room.join(room, self(), "key-a", offer)
    refute answer["sdp"] =~ "m=video"
    PeerConnection.stop(pc)
  end

  test "with screens off the video lines are refused and voice works" do
    Application.put_env(:den, :screen, false)
    on_exit(fn -> Application.delete_env(:den, :screen) end)
    {:ok, room} = Den.Voice.ensure("screen-off")
    {pc, offer} = device(video: true)
    {:ok, answer} = Den.Voice.Room.join(room, self(), "key-a", offer)
    assert for([k, _, dir] <- media(answer["sdp"]), k == "video", do: dir) |> Enum.all?(&(&1 == "inactive"))
    PeerConnection.stop(pc)
  end
```

The existing `device/0` callers keep working (`device()` has no video).

- [ ] **Step 2: Run** `mix test test/den/voice_test.exs`. Expected: the first and third tests fail (no `m=video` with a port in the answer).

- [ ] **Step 3: Implement** in `answer/4`:

```elixir
    video = if Den.Voice.screen?(), do: [:vp8], else: []
    case PeerConnection.start_link(ice_port_range: ports, ice_ip_filter: &Den.Voice.ip_ok?/1, host_to_srflx_ip_mapper: &Den.Voice.srflx/1, audio_codecs: [:opus], video_codecs: video, rtp_header_extensions: exts, controlling_process: self()) do
```

After the audio slots are set:

```elixir
          # Video: the device's screen up, the stage down, when screens are on
          # and the page offered both; otherwise any video line stays unused.
          {screen_up?, stage} =
            case PeerConnection.get_transceivers(pc) |> Enum.filter(&(&1.kind == :video)) do
              [vup, vdown | rest] when video != [] ->
                :ok = PeerConnection.set_transceiver_direction(pc, vup.id, :recvonly)
                track = MediaStreamTrack.new(:video)
                :ok = PeerConnection.set_transceiver_direction(pc, vdown.id, :sendonly)
                :ok = PeerConnection.replace_track(pc, vdown.sender.id, track)
                for t <- rest, do: :ok = PeerConnection.set_transceiver_direction(pc, t.id, :inactive)
                {true, track.id}

              vs ->
                for t <- vs, do: :ok = PeerConnection.set_transceiver_direction(pc, t.id, :inactive)
                {false, nil}
            end
```

The peer map gains `screen_up: nil, screens: screen_up?, stage: stage, stage_ready: false`. Relay peers get `screen_up: nil, screens: false, stage: nil, stage_ready: false`. `on_pc({:track, …})` tells audio from video:

```elixir
  defp on_pc({:track, %MediaStreamTrack{id: id, kind: :audio}}, channel, peer, s), do: put_in(s.peers[channel], %{peer | up: id})
  defp on_pc({:track, %MediaStreamTrack{id: id, kind: :video}}, channel, peer, s), do: put_in(s.peers[channel], %{peer | screen_up: id})
```

The audio line is still `[up | downs] = … filter(&(&1.kind == :audio))`. Video lines are not counted there.

- [ ] **Step 4: Run** `mix test test/den/voice_test.exs`. Expected: all pass, the earlier voice tests included.
- [ ] **Step 5: Commit** "Voice: each connection can carry a screen up and a stage down (VP8)".

### Task 3: Who presents

**Files:**
- Modify: `server/lib/den/voice.ex` (`Den.Voice.Room`: `present/2`, `unpresent/2`, `stop_screen/3`; `presenter` in state; `drop/2` and join tell the current presenter)
- Modify: `server/lib/den_web/channels/voice_channel.ex` (`present`, `unpresent`, `stop_screen`)
- Modify: `server/lib/den_web/throttle.ex` (limits: `"present" => 5, "unpresent" => 5, "stop_screen" => 5`)
- Test: `server/test/den/voice_test.exs`, `server/test/den_web/moderation_test.exs`

**Interfaces:**
- Consumes: peer fields from Task 2.
- Produces:
  - `Den.Voice.Room.present(pid, channel) :: :ok | {:error, :busy, key} | {:error, :unsupported} | {:error, :off}`
  - `unpresent(pid, channel) :: :ok`
  - `stop_screen(pid, by_key) :: :ok | {:error, :none}`
  - every peer is sent `{:voice, "screen", %{key: key | nil}}` when the presenter changes, and once on joining
  - the presenter is sent `{:voice, "screen_stopped", %{by: key | nil, reason: "moderator" | "too fast"}}`
  - state `presenter: channel | nil`, `screen: %{munger, key_wanted_at, bytes, since}`

- [ ] **Step 1: Failing tests**

```elixir
  test "one presenter at a time; the stage is freed when they stop or leave" do
    {:ok, room} = Den.Voice.ensure("screen-who")
    {pc_a, offer_a} = device(video: true)
    {pc_b, offer_b} = device(video: true)
    b = spawn_link(fn -> receive do: (:stop -> :ok) end)
    {:ok, _} = Den.Voice.Room.join(room, self(), "key-a", offer_a)
    assert_receive {:voice, "screen", %{key: nil}}
    {:ok, _} = Den.Voice.Room.join(room, b, "key-b", offer_b)

    assert :ok = Den.Voice.Room.present(room, self())
    assert_receive {:voice, "screen", %{key: "key-a"}}
    assert {:error, :busy, "key-a"} = Den.Voice.Room.present(room, b)

    Den.Voice.Room.unpresent(room, self())
    assert_receive {:voice, "screen", %{key: nil}}
    assert :ok = Den.Voice.Room.present(room, b)
    assert_receive {:voice, "screen", %{key: "key-b"}}

    # The presenter's process going away frees the stage.
    send(b, :stop)
    assert_receive {:voice, "screen", %{key: nil}}, 2_000
    PeerConnection.stop(pc_a)
    PeerConnection.stop(pc_b)
  end

  test "a device without a screen line, a relayed device, or screens off cannot present" do
    {:ok, room} = Den.Voice.ensure("screen-cannot")
    {pc, offer} = device()
    {:ok, _} = Den.Voice.Room.join(room, self(), "key-a", offer)
    assert {:error, :unsupported} = Den.Voice.Room.present(room, self())
    r = spawn_link(fn -> receive do: (:stop -> :ok) end)
    :ok = Den.Voice.Room.join_relay(room, r, "key-r")
    assert {:error, :unsupported} = Den.Voice.Room.present(room, r)
    Application.put_env(:den, :screen, false)
    on_exit(fn -> Application.delete_env(:den, :screen) end)
    assert {:error, :off} = Den.Voice.Room.present(room, self())
    send(r, :stop)
    PeerConnection.stop(pc)
  end

  test "stop_screen ends a presentation, tells the presenter who, and is logged" do
    {:ok, room} = Den.Voice.ensure("screen-stop")
    {pc, offer} = device(video: true)
    {:ok, _} = Den.Voice.Room.join(room, self(), "key-a", offer)
    :ok = Den.Voice.Room.present(room, self())
    assert :ok = Den.Voice.Room.stop_screen(room, "key-b")
    assert_receive {:voice, "screen_stopped", %{by: "key-b", reason: "moderator"}}
    assert_receive {:voice, "screen", %{key: nil}}
    assert {:error, :none} = Den.Voice.Room.stop_screen(room, "key-b")
    PeerConnection.stop(pc)
  end
```

In `moderation_test.exs`, through the channel. The file's setup gives
devices `ka` and `kb` in General and an owner `@owner` who is neither of them:

```elixir
  defp voice_offer do
    alias ExWebRTC.{MediaStreamTrack, PeerConnection, SessionDescription}
    {:ok, pc} = PeerConnection.start(audio_codecs: [:opus], video_codecs: [:vp8])
    {:ok, _} = PeerConnection.add_track(pc, MediaStreamTrack.new(:audio))
    [up] = PeerConnection.get_transceivers(pc)
    :ok = PeerConnection.set_transceiver_direction(pc, up.id, :sendonly)
    for _ <- 1..Den.Voice.slots(), do: {:ok, _} = PeerConnection.add_transceiver(pc, :audio, direction: :recvonly)
    {:ok, _} = PeerConnection.add_track(pc, MediaStreamTrack.new(:video))
    v = PeerConnection.get_transceivers(pc) |> Enum.find(&(&1.kind == :video))
    :ok = PeerConnection.set_transceiver_direction(pc, v.id, :sendonly)
    {:ok, _} = PeerConnection.add_transceiver(pc, :video, direction: :recvonly)
    {:ok, offer} = PeerConnection.create_offer(pc)
    :ok = PeerConnection.set_local_description(pc, offer)
    {pc, SessionDescription.to_json(offer)}
  end

  test "only a device that may kick stops someone's screen, and it is in the audit log", %{ka: ka, kb: kb} do
    {:ok, _, va} = subscribe_and_join(device(ka), DenWeb.VoiceChannel, "voice:general")
    {:ok, _, vb} = subscribe_and_join(device(kb), DenWeb.VoiceChannel, "voice:general")
    {pa, oa} = voice_offer()
    {pb, ob} = voice_offer()
    ref = push(va, "offer", oa)
    assert_reply ref, :ok, _, 5000
    ref = push(vb, "offer", ob)
    assert_reply ref, :ok, _, 5000
    ref = push(vb, "present", %{})
    assert_reply ref, :ok
    ref = push(va, "stop_screen", %{})
    assert_reply ref, :error, %{reason: "You may not stop screen sharing here."}
    {:ok, mods} = Mod.role_save(@owner, %{"name" => "Stage", "position" => 5, "perms" => ["kick"]})
    :ok = Mod.role_give(@owner, ka, mods.id)
    ref = push(va, "stop_screen", %{})
    assert_reply ref, :ok
    assert_push "screen_stopped", %{by: ^ka, reason: "moderator"}
    assert Enum.any?(Mod.audit(), &(&1.action == "screen_stop" and &1.target == kb))
    ExWebRTC.PeerConnection.stop(pa)
    ExWebRTC.PeerConnection.stop(pb)
  end
```

- [ ] **Step 2: Run** `mix test test/den/voice_test.exs test/den_web/moderation_test.exs`. Expected: fail (no `present/2`).

- [ ] **Step 3: Implement**

```elixir
  def present(pid, channel), do: GenServer.call(pid, {:present, channel})
  def unpresent(pid, channel), do: GenServer.cast(pid, {:unpresent, channel})
  def stop_screen(pid, by), do: GenServer.call(pid, {:stop_screen, by})
```

State in `init/1`: `presenter: nil, screen: fresh_screen()` with

```elixir
  defp fresh_screen, do: %{munger: ExWebRTC.RTP.Munger.new(:vp8, 90_000), live: false, key_at: nil, bytes: 0, since: System.monotonic_time(:millisecond)}
```

```elixir
  def handle_call({:present, channel}, _from, s) do
    p = s.peers[channel]
    cond do
      not Den.Voice.screen?() -> {:reply, {:error, :off}, s}
      is_nil(p) or not p.screens -> {:reply, {:error, :unsupported}, s}
      s.presenter == channel -> {:reply, :ok, s}
      s.presenter -> {:reply, {:error, :busy, s.peers[s.presenter].key}, s}
      true ->
        s = %{s | presenter: channel, screen: %{fresh_screen() | munger: Munger.update(s.screen.munger)}}
        s = Map.update!(s, :peers, &Map.new(&1, fn {c, q} -> {c, %{q | stage_ready: false}} end))
        {:reply, :ok, s |> tell_screen() |> want_key()}
    end
  end

  def handle_call({:stop_screen, by}, _from, %{presenter: nil} = s), do: {:reply, {:error, :none}, s}

  def handle_call({:stop_screen, by}, _from, s) do
    send(s.presenter, {:voice, "screen_stopped", %{by: by, reason: "moderator"}})
    Den.Mod.log(by, "screen_stop", s.peers[s.presenter].key, nil)
    {:reply, :ok, end_screen(s)}
  end

  def handle_cast({:unpresent, channel}, %{presenter: channel} = s), do: {:noreply, end_screen(s)}
  def handle_cast({:unpresent, _}, s), do: {:noreply, s}

  defp end_screen(s), do: %{s | presenter: nil} |> tell_screen()

  defp tell_screen(s) do
    key = s.presenter && s.peers[s.presenter].key
    for {c, _} <- s.peers, do: send(c, {:voice, "screen", %{key: key}})
    s
  end
```

`present` keeps the room's munger and marks it updated
(`Munger.update/1`), so viewers' numbering continues across presenters;
the rest of `screen` starts fresh. In `answer/4` and `join_relay`, after
`roster()`, send the joining channel `{:voice, "screen", %{key: key}}` with
the current presenter's key, or `nil`. In `drop/2`, the `{p, rest}` branch
starts with

```elixir
        s = if s.presenter == channel, do: end_screen(s), else: s
```

and then goes on as now, using `rest` for the remaining peers.

`want_key/1` is defined in Task 4. Until then, define it as
`defp want_key(s), do: s` so this task compiles.

Channel:

```elixir
  defp handle("present", _, socket) do
    with pid when is_pid(pid) <- socket.assigns[:room_pid] do
      case Den.Voice.Room.present(pid, self()) do
        :ok -> {:reply, :ok, socket}
        {:error, :busy, key} -> {:reply, {:error, %{reason: "busy", key: key}}, socket}
        {:error, :off} -> {:reply, {:error, %{reason: "Screen sharing is off on this den."}}, socket}
        {:error, :unsupported} -> {:reply, {:error, %{reason: "Screen sharing needs a direct connection. This network only lets voice through."}}, socket}
      end
    else
      _ -> {:reply, {:error, %{reason: "Join voice first."}}, socket}
    end
  end

  defp handle("unpresent", _, socket) do
    if pid = socket.assigns[:room_pid], do: Den.Voice.Room.unpresent(pid, self())
    {:reply, :ok, socket}
  end

  defp handle("stop_screen", _, socket) do
    cond do
      not Den.Mod.can?(socket.assigns.key, "kick") -> {:reply, {:error, %{reason: "You may not stop screen sharing here."}}, socket}
      pid = socket.assigns[:room_pid] ->
        case Den.Voice.Room.stop_screen(pid, socket.assigns.key) do
          :ok -> {:reply, :ok, socket}
          {:error, :none} -> {:reply, {:error, %{reason: "Nobody is sharing their screen."}}, socket}
        end
      true -> {:reply, {:error, %{reason: "Join voice first."}}, socket}
    end
  end
```

Fix the third server test's awkward first assert to the plain form
`assert :ok = Den.Voice.Room.stop_screen(room, "key-b")`. The room-level
call does not check permissions; the channel does.

- [ ] **Step 4: Run** `mix test test/den/voice_test.exs test/den_web/moderation_test.exs`. Expected: pass.
- [ ] **Step 5: Commit** "Voice: one presenter at a time; a moderator who may kick can stop them (audit screen_stop)".

### Task 4: Forwarding the screen

**Files:**
- Modify: `server/lib/den/voice.ex` (`on_pc` for video RTP and RTCP, `want_key/1`, guard)
- Test: `server/test/den/voice_test.exs` (`live_device/4` gains a screen)

**Interfaces:**
- Consumes: `presenter`, `screen` and peer fields from Tasks 2 and 3.
- Produces: viewers receive the presenter's packets on their stage track, numbered as one stream. `{:show, n, key?}` in the test device sends VP8 packets.

- [ ] **Step 1: Failing tests.** Extend `live_device/3` to `live_device(room, key, test, video \\ false)`:

```elixir
      {:ok, pc} = PeerConnection.start_link(audio_codecs: [:opus], video_codecs: if(video, do: [:vp8], else: []), rtp_header_extensions: exts)
      # … audio as now, then:
      {screen, stage} =
        if video do
          vt = MediaStreamTrack.new(:video)
          {:ok, _} = PeerConnection.add_track(pc, vt)
          v = PeerConnection.get_transceivers(pc) |> Enum.find(&(&1.kind == :video))
          :ok = PeerConnection.set_transceiver_direction(pc, v.id, :sendonly)
          {:ok, st} = PeerConnection.add_transceiver(pc, :video, direction: :recvonly)
          {vt.id, st.receiver.track.id}
        else
          {nil, nil}
        end
```

The loop's state gains `screen: screen, stage: stage, vseq: 0`. The `mids`
map only takes audio receivers (`t.kind == :audio`). New receive clauses
go before the generic `{:rtp, …}` one:

```elixir
      {:ex_webrtc, _, {:rtp, track, _, packet}} when track == d.stage ->
        send(d.test, {d.key, :stage, packet.sequence_number, ExWebRTC.RTP.VP8.keyframe?(packet)})
        loop(d)

      {:ex_webrtc, _, {:rtcp, packets}} ->
        for {_, %ExRTCP.Packet.PayloadFeedback.PLI{}} <- packets, do: send(d.test, {d.key, :pli})
        loop(d)

      :pli ->
        PeerConnection.send_pli(d.pc, d.stage)
        loop(d)

      {:show_big, n} ->
        for i <- 1..n do
          packet = ExRTP.Packet.new(<<0x10, 0x01, 0, 0>> <> :binary.copy(<<7>>, 1_196), payload_type: 96, sequence_number: d.vseq + i, timestamp: (d.vseq + i) * 3000, ssrc: 2)
          PeerConnection.send_rtp(d.pc, d.screen, packet)
          Process.sleep(5)
        end
        loop(%{d | vseq: d.vseq + n})
```

Also add:

```elixir
      {:show, n, first_key} ->
        for i <- 1..n do
          key = first_key and i == 1
          # VP8 payload descriptor (S=1, PID 0), then a frame whose first byte's bit 0 is 0 on a keyframe.
          payload = <<0x10, if(key, do: 0x00, else: 0x01), 0, 0>> <> :binary.copy(<<7>>, 200)
          packet = ExRTP.Packet.new(payload, payload_type: 96, sequence_number: d.vseq + i, timestamp: (d.vseq + i) * 6000, ssrc: 2, marker: true)
          PeerConnection.send_rtp(d.pc, d.screen, packet)
          Process.sleep(10)
        end
        loop(%{d | vseq: d.vseq + n})
```

Test:

```elixir
  test "viewers get only the presenter's screen, from a keyframe, as one stream; the presenter gets none" do
    {:ok, room} = Den.Voice.ensure("screen-fwd")
    devs = for k <- ~w(key-a key-b key-c), into: %{}, do: {k, live_device(room, k, self(), true)}
    for k <- Map.keys(devs), do: assert_receive({^k, :connected}, 15_000)
    on_exit(fn -> for {_, d} <- devs, do: send(d, :stop) end)

    :ok = Den.Voice.Room.present(room, devs["key-a"])
    # Not the presenter: dropped.
    send(devs["key-b"], {:show, 5, true})
    # The presenter: a few delta frames first (no viewer may start on them), then a keyframe.
    send(devs["key-a"], {:show, 3, false})
    send(devs["key-a"], {:show, 10, true})

    [first | _] = got = for _ <- 1..10, do: (assert_receive {"key-b", :stage, seq, key?}, 3_000; {seq, key?})
    assert elem(first, 1), "a viewer starts on a keyframe"
    seqs = Enum.map(got, &elem(&1, 0))
    assert seqs == Enum.to_list(hd(seqs)..(hd(seqs) + 9)), "one stream"
    refute_received {"key-a", :stage, _, _}
    # key-c sees the same.
    assert_receive {"key-c", :stage, _, true}, 3_000
  end

  test "presenting and a late viewer ask the presenter for a keyframe; viewers' asks at most once a second" do
    {:ok, room} = Den.Voice.ensure("screen-pli")
    a = live_device(room, "key-a", self(), true)
    assert_receive {"key-a", :connected}, 15_000
    on_exit(fn -> send(a, :stop) end)
    :ok = Den.Voice.Room.present(room, a)
    assert_receive {"key-a", :pli}, 2_000
    Process.sleep(1_100)
    d = live_device(room, "key-d", self(), true)
    on_exit(fn -> send(d, :stop) end)
    assert_receive {"key-d", :connected}, 15_000
    assert_receive {"key-a", :pli}, 1_000
    # The viewer's decoder asks three times at once: one more PLI, a second later.
    for _ <- 1..3, do: send(d, :pli)
    refute_receive {"key-a", :pli}, 600
    assert_receive {"key-a", :pli}, 1_000
    refute_receive {"key-a", :pli}, 1_200
  end

  test "a presenter sending far above the bitrate is stopped" do
    Application.put_env(:den, :screen_bitrate, 300_000)
    on_exit(fn -> Application.delete_env(:den, :screen_bitrate) end)
    {:ok, room} = Den.Voice.ensure("screen-fast")
    a = live_device(room, "key-a", self(), true)
    on_exit(fn -> send(a, :stop) end)
    assert_receive {"key-a", :connected}, 15_000
    :ok = Den.Voice.Room.present(room, a)
    # 400 × 1,200 bytes is 3.84 Mbit in about 2 s, over 2 × 0.3 Mbps × 5 s (3 Mbit).
    send(a, {:show_big, 400})
    assert_receive {"key-a", "screen_stopped", %{by: nil, reason: "too fast"}}, 8_000
    assert_receive {"key-a", "screen", %{key: nil}}, 1_000
  end
```

- [ ] **Step 2: Run** `mix test test/den/voice_test.exs`. Expected: fail (no stage packets).

- [ ] **Step 3: Implement**

```elixir
  alias ExWebRTC.RTP.{Munger, VP8}
  alias ExRTCP.Packet.PayloadFeedback.{PLI, FIR}

  # The presenter's screen: through the room's one munger, to every viewer
  # that has had a keyframe. Anyone else's video goes nowhere.
  defp on_pc({:rtp, id, _rid, packet}, channel, %{screen_up: id} = _peer, %{presenter: channel} = s) do
    now = System.monotonic_time(:millisecond)
    s = count(s, byte_size(packet.payload), now)
    if s.presenter != channel do
      s
    else
      key? = VP8.keyframe?(packet)
      if not s.screen.live and not key? do
        s
      else
        {packet, munger} = Munger.munge(s.screen.munger, packet)
        s = put_in(s.screen, %{s.screen | munger: munger, live: true})
        Enum.reduce(s.peers, s, fn {c, q}, s ->
          cond do
            c == channel or is_nil(q.stage) -> s
            q.stage_ready -> (PeerConnection.send_rtp(q.pc, q.stage, packet); s)
            key? -> (PeerConnection.send_rtp(q.pc, q.stage, packet); put_in(s.peers[c].stage_ready, true))
            true -> s
          end
        end)
      end
    end
  end

  defp on_pc({:rtp, id, _rid, _packet}, _channel, %{screen_up: id}, s), do: s

  # A viewer's decoder asks for a keyframe: to the presenter, within the limit.
  defp on_pc({:rtcp, packets}, _channel, _peer, s) do
    if Enum.any?(packets, fn {_, p} -> is_struct(p, PLI) or is_struct(p, FIR) end), do: want_key(s), else: s
  end
```

Place these clauses before the audio `{:rtp, id, …}` clause. That clause
matches `%{up: id}`, and since `up` and `screen_up` are different ids, the
order only matters for readability. Keep the catch-all `on_pc(_, _, _, s)`
last.

```elixir
  @key_every 1_000

  defp want_key(%{presenter: nil} = s), do: s

  defp want_key(s) do
    now = System.monotonic_time(:millisecond)
    p = s.peers[s.presenter]
    cond do
      is_nil(p.screen_up) -> s
      s.screen.key_at && now - s.screen.key_at < @key_every ->
        # Asked within the second: once more when it is up.
        unless s.screen[:again], do: Process.send_after(self(), :want_key, @key_every - (now - s.screen.key_at))
        put_in(s.screen[:again], true)
      true ->
        PeerConnection.send_pli(p.pc, p.screen_up)
        put_in(s.screen, Map.merge(s.screen, %{key_at: now, again: false}))
    end
  end

  def handle_info(:want_key, s), do: {:noreply, want_key(put_in(s.screen[:again], false))}

  # The guard: twice the bitrate, over 5 seconds, ends the presentation.
  defp count(s, n, now) do
    sc = s.screen
    {bytes, since} = if now - sc.since > 5_000, do: {n, now}, else: {sc.bytes + n, sc.since}
    s = put_in(s.screen, %{sc | bytes: bytes, since: since})
    if bytes * 8 > 2 * Den.Voice.screen_bitrate() * 5 do
      send(s.presenter, {:voice, "screen_stopped", %{by: nil, reason: "too fast"}})
      end_screen(s)
    else
      s
    end
  end
```

`fresh_screen/0` gains `again: false`. `want_key/1` is called on present
(Task 3), and when a viewer connects
(`on_pc({:connection_state_change, :connected}, …)` sets `stage_ready:
false`, then `want_key`). The Task 3 stub `defp want_key(s), do: s` is
removed.

The guard counts in 5-second windows, not a sliding average. That is enough
to stop a device multiplying its upload, and it is what the test checks.

- [ ] **Step 4: Run** `mix test test/den/voice_test.exs`. Expected: pass. Then `mix test`. Expected: all pass.
- [ ] **Step 5: Commit** "Voice: the presenter's screen to every viewer from a keyframe, one stream; keyframes asked at most once a second; a presenter far over the bitrate is stopped".

### Task 5: Sealing screen frames

**Files:**
- Create: `web/media-seal.js` (pure: `clearLength`, `sealFrame`, `openFrame`, `Window`)
- Modify: `web/voice-worker.js` (a module worker importing it; screen keys; `seal-screen` / `open-screen` modes)
- Modify: `web/build.sh` (copy `media-seal.js` to `den/`)
- Modify: `web/den.js` (`new Worker(…, { type: 'module' })`)
- Test: `web/tests/den.test.js`

**Interfaces:**
- Produces:
  - `clearLength(bytes: Uint8Array, kind: 'audio' | 'screen') :: number` (0 for audio; 10 or 3 for screen; throws if the frame is shorter)
  - `sealFrame(key, index, counter: bigint, data: Uint8Array, clear: number) :: Promise<Uint8Array>`
  - `openFrame(key, data: Uint8Array, clear: number) :: Promise<{ plain: Uint8Array, counter: bigint, index: number }>`
  - `Window` (`fresh(sender, n) :: boolean`)
  - Worker messages: `{ keys, screenKeys, current, me, presenter }`; transform options `mode: 'seal' | 'open' | 'seal-screen' | 'open-screen'`.

Frame layout: `[clear prefix][ciphertext ‖ tag][iv 12][counter 8][key index 1]`.
The additional data is `clear prefix ‖ last 9 bytes`. For audio the clear
prefix is empty, so the bytes are exactly today's.

- [ ] **Step 1: Failing test**

```js
test('screen frames keep VP8 bytes in the clear, sealed to them; voice frames are as before', async () => {
  const { webcrypto } = await import('node:crypto');
  globalThis.crypto ??= webcrypto;
  const { clearLength, sealFrame, openFrame, Window } = await import('../dist/den/media-seal.js');
  const key = await crypto.subtle.generateKey({ name: 'AES-GCM', length: 256 }, false, ['encrypt', 'decrypt']);
  const other = await crypto.subtle.generateKey({ name: 'AES-GCM', length: 256 }, false, ['encrypt', 'decrypt']);
  const keyframe = Uint8Array.from({ length: 300 }, (_, i) => (i === 0 ? 0x10 : i));
  const delta = Uint8Array.from({ length: 300 }, (_, i) => (i === 0 ? 0x11 : i));
  assert.equal(clearLength(keyframe, 'screen'), 10);
  assert.equal(clearLength(delta, 'screen'), 3);
  assert.equal(clearLength(keyframe, 'audio'), 0);
  assert.throws(() => clearLength(new Uint8Array(2), 'screen'));
  const n = BigInt(Date.now()) * 1000n;
  for (const f of [keyframe, delta]) {
    const c = clearLength(f, 'screen');
    const sealed = await sealFrame(key, 7, n, f, c);
    assert.deepEqual(sealed.subarray(0, c), f.subarray(0, c), 'the clear prefix is as it was');
    assert.notDeepEqual(sealed.subarray(c, c + 20), f.subarray(c, c + 20), 'the rest is not');
    const back = await openFrame(key, sealed, c);
    assert.deepEqual(back.plain, f);
    assert.equal(back.index, 7);
    const bent = sealed.slice(); bent[1] ^= 1;
    await assert.rejects(openFrame(key, bent, c), 'a changed clear byte does not open');
    await assert.rejects(openFrame(other, sealed, c), "another sender's key does not open");
  }
  // Voice: no clear bytes, the layout of before.
  const voice = Uint8Array.from({ length: 80 }, (_, i) => i);
  const sv = await sealFrame(key, 1, n, voice, 0);
  assert.equal(sv.length, voice.length + 16 + 21);
  assert.deepEqual((await openFrame(key, sv, 0)).plain, voice);
  const w = new Window();
  assert.ok(w.fresh('s', 10n) && w.fresh('s', 12n) && w.fresh('s', 11n));
  assert.ok(!w.fresh('s', 11n), 'a counter seen is refused');
});
```

- [ ] **Step 2: Run** `cd web && npm test`. Expected: fails ("Cannot find module …/dist/den/media-seal.js").

- [ ] **Step 3: Implement** `web/media-seal.js`:

```js
// One media frame sealed for the den to forward and not read: AES-GCM under
// the sender's key, with the counter and key index after it. Screen frames
// keep the start of the VP8 frame in the clear, which WebRTC needs to
// packetise them: 10 bytes on a keyframe, 3 otherwise. Those bytes are part
// of what the tag covers, so they cannot be changed either.
export const TAIL = 21;
export const TAG = 16;

export function clearLength(bytes, kind) {
  if (kind !== 'screen') return 0;
  const n = (bytes[0] & 1) === 0 ? 10 : 3;
  if (bytes.length < n) throw new Error('short');
  return n;
}

export async function sealFrame(key, index, counter, data, clear) {
  const head = new Uint8Array(9);
  const v = new DataView(head.buffer);
  v.setUint32(0, Number(counter >> 32n));
  v.setUint32(4, Number(counter & 0xffffffffn));
  head[8] = index;
  const prefix = data.subarray(0, clear);
  const iv = crypto.getRandomValues(new Uint8Array(12));
  const ct = new Uint8Array(await crypto.subtle.encrypt({ name: 'AES-GCM', iv, additionalData: join(prefix, head) }, key, data.subarray(clear)));
  const out = new Uint8Array(clear + ct.length + TAIL);
  out.set(prefix);
  out.set(ct, clear);
  out.set(iv, clear + ct.length);
  out.set(head, clear + ct.length + 12);
  return out;
}

export async function openFrame(key, d, clear) {
  if (d.length < clear + TAG + TAIL) throw new Error('short');
  const head = d.slice(d.length - 9);
  const v = new DataView(head.buffer);
  const counter = BigInt(v.getUint32(0)) * 2n ** 32n + BigInt(v.getUint32(4));
  const iv = d.subarray(d.length - TAIL, d.length - 9);
  const prefix = d.subarray(0, clear);
  const body = new Uint8Array(await crypto.subtle.decrypt({ name: 'AES-GCM', iv, additionalData: join(prefix, head) }, key, d.subarray(clear, d.length - TAIL)));
  return { plain: join(prefix, body), counter, index: head[8] };
}

/// The key index of a sealed frame, read before choosing the key.
export const indexOf = (d) => d[d.length - 1];

function join(a, b) {
  const out = new Uint8Array(a.length + b.length);
  out.set(a);
  out.set(b, a.length);
  return out;
}

/// A sliding window of 64 frames per sender: newer counters move it on, a
/// late frame inside it is taken once, anything older is refused.
export class Window {
  constructor(size = 64n) { this.size = size; this.seen = new Map(); }
  fresh(sender, n) {
    const s = this.seen.get(sender);
    if (!s) { this.seen.set(sender, { max: n, mask: 1n }); return true; }
    if (n > s.max) {
      const shift = n - s.max;
      s.mask = shift >= this.size ? 1n : ((s.mask << shift) | 1n) & ((1n << this.size) - 1n);
      s.max = n;
      return true;
    }
    const back = s.max - n;
    if (back >= this.size) return false;
    const bit = 1n << back;
    if (s.mask & bit) return false;
    s.mask |= bit;
    return true;
  }
}
```

`voice-worker.js` becomes a module (`import { clearLength, sealFrame,
openFrame, indexOf, Window } from './media-seal.js';`). `seal`/`open` take a
`kind`:

```js
let screenKeys = new Map(); // key index → Map(sender → CryptoKey), 'den screen'
let presenter = '';
const voiceSeen = new Window();
const screenSeen = new Window();

async function seal(frame, kind = 'audio') {
  const ks = kind === 'screen' ? screenKeys : keys;
  const key = ks.get(current)?.get(me);
  if (!key) throw new Error('no key');
  counter = Math.max(counter + 1, Date.now() * 1000);
  const data = new Uint8Array(frame.data);
  frame.data = (await sealFrame(key, current, BigInt(counter), data, clearLength(data, kind))).buffer;
  sealed++;
  return frame;
}

async function open(frame, slot, kind = 'audio') {
  const d = new Uint8Array(frame.data);
  const sender = kind === 'screen' ? presenter : slots.get(slot);
  const key = sender && (kind === 'screen' ? screenKeys : keys).get(indexOf(d))?.get(sender);
  if (!key) throw new Error('no key');
  const { plain, counter: n } = await openFrame(key, d, clearLength(d, kind));
  if (Math.abs(Number(n / 1000n) - Date.now()) > STALE_MS) throw new Error('stale');
  if (!(kind === 'screen' ? screenSeen : voiceSeen).fresh(sender, n)) throw new Error('replay');
  frame.data = plain.buffer;
  opened++;
  return frame;
}
```

`pipe` maps modes: `'seal'` → `seal(f)`, `'open'` → `open(f, slot)`,
`'seal-screen'` → `seal(f, 'screen')`, `'open-screen'` → `open(f, null, 'screen')`.
The keys message also sets `screenKeys` and `presenter` when they are
present. The relayed `op` path uses `'audio'`. Delete the old in-file
`fresh()` and `seen`.

`clearLength` reads the prefix from the plain frame when sealing, and from
the sealed frame when opening. They are the same bytes, which is the point.

`build.sh`: `cp "$WEB/media-seal.js" "$OUT/den/media-seal.js"`. In
`den.js`: `new Worker(new URL('./den/voice-worker.js', import.meta.url), { type: 'module' })`.

- [ ] **Step 4: Run** `cd web && npm test`. Expected: pass. Then run the voice part of `den.e2e.mjs` (Task 8's command): voice still opens frames (`opened` rises, `failed` stays 0).
- [ ] **Step 5: Commit** "Page: media frames sealed by one module; screen frames keep VP8's first bytes in the clear, under the tag".

### Task 6: The page's screen client

**Files:**
- Modify: `web/den.js` (`SCREEN_LABEL`, `screenKey`, `Voice`: transceivers, `refreshKey` with screen keys, `present()`, `stopPresenting()`, `stopScreen()`, `screen` events, `stage` stream)

**Interfaces:**
- Consumes: the join reply's `screen` and `screen_bitrate` (Task 1); `present`, `unpresent` and `stop_screen` (Task 3); worker modes (Task 5).
- Produces, for the UI (Task 7):
  - `voice.screen` ({ key } | null), `voice.presenting` (boolean), `voice.stage` (MediaStream | null)
  - `voice.canPresent` (boolean: `rtc` mode, the den allows it, `getDisplayMedia` exists)
  - `await voice.present()` throws Error('busy:' + key) or Error(reason)
  - `await voice.stopPresenting()`, `await voice.stopScreen()`
  - `onSlots` is also called on screen changes; `voice.stoppedBy` = { by, reason } after a stop

- [ ] **Step 1:** Add `export const SCREEN_LABEL = 'den screen';` beside `VOICE_LABEL`, and:

```js
  async screenKey(room, sender) {
    const raw = this.den.export(room, `${SCREEN_LABEL}|${sender}`, 32);
    return crypto.subtle.importKey('raw', raw, 'AES-GCM', false, ['encrypt', 'decrypt']);
  }
```

- [ ] **Step 2:** In `refreshKey`, build `screenKeys` for the same senders
  (`me`, the roster, the presenter) and post `{ keys, screenKeys, current,
  me, presenter: this.screen?.key || '' }`. Do the same in the `dropOld`
  timeout. It is the same loop with `screenKey`.

- [ ] **Step 3:** In `start()`: keep the join reply, `const joined = await
  new Promise(…receive('ok', resolve)…)`, then `this.screenAllowed = !!joined?.screen`
  and `this.screenBitrate = joined?.screen_bitrate || 2_500_000`. Add these handlers:

```js
    ch.on('screen', ({ key }) => {
      this.screen = typeof key === 'string' ? { key } : null;
      if (!this.screen && this.presenting) this.endCapture();
      this.refreshKey().catch(() => {});
      this.onSlots(this.people);
    });
    ch.on('screen_stopped', (m) => {
      this.stoppedBy = { by: m?.by || null, reason: m?.reason || '' };
      this.endCapture();
      this.onSlots(this.people);
    });
```

- [ ] **Step 4:** In `startRtc()`, after the audio downs, when `this.screenAllowed`:

```js
      this.screenUp = pc.addTransceiver('video', { direction: 'sendonly' });
      this.transform(this.screenUp.sender, 'seal-screen');
      this.stageT = pc.addTransceiver('video', { direction: 'recvonly' });
      this.transform(this.stageT.receiver, 'open-screen');
      // VP8 only: the den forwards VP8, and the clear prefix is VP8's.
      const vp8 = RTCRtpReceiver.getCapabilities?.('video')?.codecs.filter((c) => /vp8|rtx/i.test(c.mimeType));
      if (vp8?.length) for (const t of [this.screenUp, this.stageT]) t.setCodecPreferences?.(vp8);
```

`pc.ontrack` handles video: `if (e.track.kind === 'video') { this.stage =
new MediaStream([e.track]); this.onSlots(this.people); return; }` before
the audio path.

- [ ] **Step 5:** Methods:

```js
  get canPresent() {
    return this.mode === 'rtc' && !!this.screenUp && !!navigator.mediaDevices?.getDisplayMedia;
  }

  /// Share this device's screen with the voice room: the browser asks which
  /// screen, window or tab; then the den is asked for the stage.
  async present() {
    if (!this.canPresent) throw new Error('Screen sharing needs a direct connection. This network only lets voice through.');
    const cap = await navigator.mediaDevices.getDisplayMedia({ video: { frameRate: 15 }, audio: false });
    const track = cap.getVideoTracks()[0];
    try {
      await this.client.push(this.ch, 'present', {});
    } catch (e) {
      cap.getTracks().forEach((t) => t.stop());
      throw e;
    }
    track.contentHint = 'detail';
    track.onended = () => { this.stopPresenting().catch(() => {}); };
    await this.screenUp.sender.replaceTrack(track);
    const p = this.screenUp.sender.getParameters();
    p.encodings = p.encodings?.length ? p.encodings : [{}];
    Object.assign(p.encodings[0], { maxBitrate: this.screenBitrate, maxFramerate: 15 });
    await this.screenUp.sender.setParameters(p).catch(() => {});
    this.capture = cap;
    this.presenting = true;
    this.stoppedBy = null;
    this.onSlots(this.people);
  }

  async stopPresenting() {
    this.endCapture();
    try { await this.client.push(this.ch, 'unpresent', {}); } catch { /* gone */ }
  }

  endCapture() {
    this.capture?.getTracks().forEach((t) => t.stop());
    this.capture = null;
    this.presenting = false;
    this.screenUp?.sender.replaceTrack(null).catch(() => {});
  }

  /// A moderator's stop for whoever presents.
  stopScreen() { return this.client.push(this.ch, 'stop_screen', {}); }
```

`client.push` rejects with the den's `reason`. For `busy`, the reply also
has `key`. Make `push` reject with `Error(\`busy:${key}\`)` when
`reason === 'busy'`: read `push` first and follow its error shape.
`leave()` and `stopRtc()` call `endCapture()`.

- [ ] **Step 6:** `cd web && npm test`. Expected: pass (no behaviour test here; the e2e in Task 8 is this task's test).
- [ ] **Step 7: Commit** "Page: share a screen from voice, watch the stage, keys per epoch".

### Task 7: Screen sharing in the page's UI

**Files:**
- Modify: `web/index.html` (stage markup, a Share screen button in `.voice-acts`)
- Modify: `web/den-ui.js` (`drawVoice`, `drawStage`, buttons)
- Modify: `web/den.css` (stage)
- Test: `web/tests/den.test.js`

- [ ] **Step 1: Failing test**

```js
test('the page has a stage for a shared screen and a button to share one', async () => {
  const html = await read('../index.html');
  for (const id of ['voice-screen', 'stage', 'stage-video', 'stage-who', 'stage-full', 'stage-stop', 'stage-note']) assert.match(html, new RegExp(`id="${id}"`), id);
  const ui = await read('../den-ui.js');
  assert.match(ui, /Share screen/);
  assert.match(ui, /is sharing their screen/);
  assert.match(ui, /Screen sharing needs a direct connection\. This network only lets voice through\./);
});
```

- [ ] **Step 2: Run** `cd web && npm test`. Expected: fail.

- [ ] **Step 3: Markup.** In `.voice-acts`, before Mute: `<button type="button" class="mini" id="voice-screen" hidden>Share screen</button>`. Between `#voice` and `.den-body`:

```html
          <section class="stage" id="stage" hidden aria-labelledby="stage-who">
            <div class="stage-bar"><span id="stage-who"></span><span class="stage-acts"><button type="button" class="mini" id="stage-full">Full screen</button><button type="button" class="mini leave" id="stage-stop" hidden>Stop</button></span></div>
            <video id="stage-video" autoplay playsinline muted></video>
            <p class="stage-note" id="stage-note" hidden></p>
          </section>
```

- [ ] **Step 4: UI logic** in `den-ui.js`. `drawVoice()` calls `drawStage()`.

```js
  function drawStage() {
    const v = den.voice;
    const s = v?.screen;
    const here = v && v.room === room;
    const btn = el('voice-screen');
    btn.hidden = !v || !v.canPresent;
    btn.textContent = v?.presenting ? 'Stop sharing' : 'Share screen';
    btn.disabled = !!(s && !v.presenting);
    btn.title = s && !v.presenting ? `${display(s.key)} is sharing` : '';
    el('stage').hidden = !(here && (s || v?.stoppedBy));
    const note = el('stage-note');
    note.hidden = true;
    if (v?.stoppedBy) {
      note.hidden = false;
      note.textContent = v.stoppedBy.reason === 'too fast' ? 'Your screen share sent more than this den allows, so it stopped.' : `${display(v.stoppedBy.by)} stopped your screen share`;
    }
    if (!s) { el('stage-video').srcObject = null; return; }
    el('stage-who').textContent = v.presenting ? 'You are sharing your screen' : `${display(s.key)} is sharing their screen`;
    el('stage-stop').hidden = v.presenting || !den.can('kick');
    if (v.mode === 'relay') { note.hidden = false; note.textContent = 'Screen sharing needs a direct connection. This network only lets voice through.'; }
    const video = el('stage-video');
    video.hidden = v.presenting || v.mode === 'relay';
    if (!video.hidden && video.srcObject !== v.stage) video.srcObject = v.stage || null;
    // In another room: the voice bar's Watch.
    el('voice-room').textContent = here || !s ? roomTitle(v.room) : `${roomTitle(v.room)} · ${display(s.key)} is sharing · Watch`;
  }
  el('voice-screen').onclick = async () => {
    const v = den.voice;
    if (!v) return;
    try {
      if (v.presenting) await v.stopPresenting(); else await v.present();
    } catch (e) {
      const m = /^busy:(.+)$/.exec(e.message);
      status(m ? `${display(m[1])} is sharing` : e.name === 'NotAllowedError' ? '' : e.message);
    }
    drawStage();
  };
  el('stage-full').onclick = () => el('stage-video').requestFullscreen?.().catch(() => {});
  el('stage-stop').onclick = async () => { try { await den.voice?.stopScreen(); } catch (e) { status(e.message); } };
```

`den.can(perm)` is how den-ui.js checks the device's own permissions. The voice bar's room button already goes to the voice room
(`go(v.room)`), so "Watch" is the same button. Call `drawStage()` when the
open room changes (where `drawRooms()` runs on `go`).

- [ ] **Step 5: Styles** in `den.css`:

```css
.stage { border-bottom: 1px solid var(--line); background: #000; display: grid; }
.stage-bar { display: flex; justify-content: space-between; align-items: center; gap: 10px; padding: 6px 12px; font: 12.5px var(--sans); color: var(--read); background: rgba(0, 0, 0, 0.6); }
.stage-acts { display: flex; gap: 6px; }
#stage-video { width: 100%; max-height: 62vh; object-fit: contain; background: #000; }
.stage-note { margin: 0; padding: 10px 12px; font: 13px var(--sans); color: var(--read); }
@media (max-width: 720px) { #stage-video { max-height: 40vh; } }
```

- [ ] **Step 6: Run** `cd web && npm test`. Expected: pass.
- [ ] **Step 7: Commit** "Page: Share screen in voice, the stage with Full screen, a moderator's Stop".

### Task 8: End to end in Chrome

**Files:**
- Modify: `web/tests/den.e2e.mjs` (a section after "Voice in General", before the relayed voice section)

**Interfaces:**
- Consumes: everything above. A and B are in General's voice; D is the
  relayed member later in the file. Moderation steps earlier in the file
  show how a moderator device is set up, so reuse that device or make A the
  owner as the moderation section does.

- [ ] **Step 1: The fake screen.** In the contexts' init script (where the file adds init scripts), add for every page:

```js
// A screen to share: a canvas whose colour changes every second (red, green, blue).
navigator.mediaDevices.getDisplayMedia = async () => {
  const c = Object.assign(document.createElement('canvas'), { width: 640, height: 360 });
  const g = c.getContext('2d');
  const colours = ['#ff0000', '#00ff00', '#0000ff'];
  const draw = () => { g.fillStyle = colours[Math.floor(Date.now() / 1000) % 3]; g.fillRect(0, 0, 640, 360); };
  draw();
  setInterval(draw, 100);
  return c.captureStream(15);
};
```

- [ ] **Step 2: Checks.** These go in three places in the file.

(a) After "Voice in General" (A and B in voice, before B leaves for the relayed test):

```js
// Screen sharing in General's voice.
ok(await until(A.p, () => !document.querySelector('#voice-screen').hidden), 'A is offered Share screen');
await A.p.click('#voice-screen');
ok(await until(B.p, () => /is sharing their screen/.test(document.querySelector('#stage-who')?.textContent || ''), null, 10000), 'B sees that A is sharing');
ok(await until(B.p, () => document.querySelector('#stage-video').videoWidth > 0, null, 15000), "B's stage shows a picture");
const colour = () => B.p.evaluate(() => {
  const v = document.querySelector('#stage-video');
  const c = Object.assign(document.createElement('canvas'), { width: v.videoWidth, height: v.videoHeight });
  const g = c.getContext('2d'); g.drawImage(v, 0, 0);
  const [r, gr, b] = g.getImageData(c.width >> 1, c.height >> 1, 1, 1).data;
  return r > 150 && gr < 100 && b < 100 ? 'red' : gr > 150 && r < 100 ? 'green' : b > 150 && r < 100 ? 'blue' : 'other';
});
const seen = new Set();
for (let i = 0; i < 40 && seen.size < 3; i++) { seen.add(await colour()); await B.p.waitForTimeout(150); }
ok(['red', 'green', 'blue'].every((c) => seen.has(c)), `B's picture follows A's screen (${[...seen]})`);
const vs = await B.p.evaluate(() => ({ failed: window.__den.voice.failed, presenter: window.__den.voice.screen?.key }));
ok(vs.failed === 0 && vs.presenter === A.me.key, 'every screen frame opened, under the key of the device the den names');
ok(await B.p.evaluate(() => document.querySelector('#voice-screen').disabled), 'B cannot share while A shares');
const busy = await B.p.evaluate(() => window.__den.voice.present().then(() => 'ok', (e) => e.message));
ok(busy.startsWith('busy:'), 'a second presenter is refused');
```

A keeps sharing, and B stays in voice over WebRTC, through the relayed
section (do not make B leave voice while A shares: move the relayed test's
"B over the den" part after (b), or let a fresh context play B's relayed
part). D then enters General, which starts a new epoch.

(b) Right after "D is in General":

```js
const before = await B.p.evaluate(() => window.__den.voice.opened);
ok(await until(B.p, ([n]) => window.__den.voice.opened > n + 30, [before], 10000), 'after a new epoch (D came in) the stage keeps opening frames');
```

and after "D is in voice through the den":

```js
ok(await until(D.p, () => /needs a direct connection/.test(document.querySelector('#stage-note')?.textContent || ''), null, 10000), 'a relayed member is told the screen needs a direct connection');
ok(await D.p.evaluate(() => document.querySelector('#voice-screen').hidden), 'and is not offered Share screen');
await A.p.click('#voice-screen');
ok(await until(B.p, () => document.querySelector('#stage').hidden, null, 5000), 'Stop sharing frees the stage');
```

(c) After "the host makes A the owner" (A may kick), with A and B both
joining General's voice again:

```js
await A.p.click('#voice-go');
await B.p.click('#voice-go');
ok(await until(B.p, () => window.__den.voice?.pc?.connectionState === 'connected', null, 15000), 'B is back in voice');
await B.p.click('#voice-screen');
ok(await until(A.p, () => !document.querySelector('#stage-stop').hidden, null, 10000), 'the owner sees Stop on B\'s screen');
await A.p.click('#stage-stop');
ok(await until(B.p, () => /stopped your screen share/.test(document.querySelector('#stage-note')?.textContent || ''), null, 5000), 'B is told who stopped their share');
ok(host(`IO.puts(Enum.any?(Den.Mod.audit(), &(&1.action == "screen_stop")))`).trim() === 'true', 'it is in the audit log');
await A.p.click('#voice-leave');
await B.p.click('#voice-leave');
```

- [ ] **Step 3: Run.** On the clean test den (`:4200`, scratchpad `clean-den/env.exs`):

```bash
cd web && ./build.sh && DEN=http://localhost:4200 DEN_ENV=<clean-den env.exs> PLAYWRIGHT_FROM=../../nap-connect node tests/den.e2e.mjs
```

Expected: all checks pass; the count grows by about 12.
- [ ] **Step 4: Commit** "e2e: a shared screen reaches a viewer, follows the presenter's picture, survives a new epoch; busy, Stop sharing, a moderator's Stop, a relayed member".

### Task 9: Docs, the site, publishing

**Files:**
- Modify: `scripts/den/root/README.md` (features: Voice line; settings table: `DEN_SCREEN`, `DEN_SCREEN_BITRATE`, with the bandwidth line)
- Modify: `scripts/den/root/THREAT_MODEL.md` (what the server sees: presenter, times, amount of video, keyframe timing, the shared picture's size)
- Modify: `scripts/den/root/DEVLOG.md` (newest first: what and why)
- Modify: `apps/epeaky-site/public/index.html` (Voice feature: screen sharing; "The server sees": who shares a screen, when, and the picture's size); `compare/` and `discord-alternative/` rows "no video or screen sharing" become "screen sharing; no video calls"

- [ ] **Step 1:** Write the docs. Use the spec's "What the server sees" list word for word in THREAT_MODEL.
- [ ] **Step 2:** Run all tests: `cd server && mix test`; `cd web && npm test`; `cd mls && cargo test`; the e2e as in Task 8.
- [ ] **Step 3:** Get a whole-branch review from a fresh reviewer, and fix what it finds.
- [ ] **Step 4:** Commit. Run `scripts/den/sync.sh` (check the diff lists only these files), then `scripts/den/sync.sh --push`. Wait for CI and the image build to go green.
- [ ] **Step 5:** Rebuild and deploy the site (`apps/epeaky-site/build.sh`, `wrangler pages deploy dist --project-name epeaky-site --branch main`).
- [ ] **Step 6:** Hand the user the update for the test den (`ssh root@159.69.252.109 'epeaky update'`, which the user runs) and the by-hand checks: Safari and Firefox as presenter and viewer, a phone as viewer, three devices on gen-epeaky.zyn-network.com.
