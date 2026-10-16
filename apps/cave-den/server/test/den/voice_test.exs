defmodule Den.VoiceTest do
  # The forwarding unit's side of a join, with ex_webrtc playing the browser:
  # one track up, eight slots down, an answer that sends on every slot, and a
  # slot map that names each other speaker.
  use ExUnit.Case, async: false
  alias ExWebRTC.{MediaStreamTrack, PeerConnection, SessionDescription}

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

  # Each media line of an SDP: its kind, port and direction.
  defp media(sdp) do
    sdp
    |> String.split(~r/\r?\nm=/)
    |> Enum.drop(1)
    |> Enum.map(fn sec ->
      [kind, port | _] = String.split(sec, " ")
      dir = Regex.run(~r/a=(sendonly|recvonly|sendrecv|inactive)/, sec, capture: :all_but_first) |> List.wrap() |> List.first()
      {kind, port, dir}
    end)
  end

  defp directions(sdp), do: Regex.scan(~r/a=(sendonly|recvonly|sendrecv|inactive)/, sdp, capture: :all_but_first) |> List.flatten()

  test "screen sharing is on by default at 2.5 Mbps, and the host can turn it off" do
    Application.delete_env(:den, :screen)
    Application.delete_env(:den, :screen_bitrate)
    assert Den.Voice.screen?()
    assert Den.Voice.screen_bitrate() == 2_500_000
    Application.put_env(:den, :screen, false)
    refute Den.Voice.screen?()
    Application.delete_env(:den, :screen)
  end

  test "an offer with a screen and a stage is answered with VP8: the den takes the screen and sends the stage" do
    {:ok, room} = Den.Voice.ensure("screen-answer")
    {pc, offer} = device(video: true)
    {:ok, answer} = Den.Voice.Room.join(room, self(), "key-a", offer)
    videos = for {"video", port, dir} <- media(answer["sdp"]), do: {port, dir}
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
    videos = for {"video", port, dir} <- media(answer["sdp"]), do: {port, dir}
    assert videos != [] and Enum.all?(videos, fn {port, dir} -> port == "0" or dir == "inactive" end), inspect(videos)
    PeerConnection.stop(pc)
  end

  test "one presenter at a time; the stage is freed when they stop or leave" do
    {:ok, room} = Den.Voice.ensure("screen-who")
    {pc_a, offer_a} = device(video: true)
    {pc_b, offer_b} = device(video: true)
    me = self()
    b = spawn_link(fn -> receive do: (:stop -> :ok) end)
    {:ok, _} = Den.Voice.Room.join(room, me, "key-a", offer_a)
    assert_receive {:voice, "screen", %{key: nil}}
    {:ok, _} = Den.Voice.Room.join(room, b, "key-b", offer_b)

    assert :ok = Den.Voice.Room.present(room, me)
    assert_receive {:voice, "screen", %{key: "key-a"}}
    assert {:error, :busy, "key-a"} = Den.Voice.Room.present(room, b)

    Den.Voice.Room.unpresent(room, me)
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

  test "stop_screen ends a presentation and tells the presenter who" do
    {:ok, room} = Den.Voice.ensure("screen-stop")
    {pc, offer} = device(video: true)
    {:ok, _} = Den.Voice.Room.join(room, self(), "key-a", offer)
    :ok = Den.Voice.Room.present(room, self())
    assert {:error, :none} = Den.Voice.Room.stop_screen(room, "key-b", "key-x")
    assert "key-a" = Den.Voice.Room.presenter(room)
    assert {:ok, "key-a"} = Den.Voice.Room.stop_screen(room, "key-b", "key-a")
    assert_receive {:voice, "screen_stopped", %{by: "key-b", reason: "moderator"}}
    assert_receive {:voice, "screen", %{key: nil}}
    assert {:error, :none} = Den.Voice.Room.stop_screen(room, "key-b", "key-a")
    assert nil == Den.Voice.Room.presenter(room)
    PeerConnection.stop(pc)
  end

  test "two devices join a room's voice and each is given the other's slot" do
    {:ok, room} = Den.Voice.ensure("voice-test")
    {pc_a, offer_a} = device()
    {pc_b, offer_b} = device()

    a = self()
    b = spawn_link(fn -> receive do: (:stop -> :ok) end)

    {:ok, answer} = Den.Voice.Room.join(room, a, "key-a", offer_a)
    assert ["recvonly" | slots] = directions(answer["sdp"])
    assert slots == List.duplicate("sendonly", Den.Voice.slots())
    :ok = PeerConnection.set_remote_description(pc_a, SessionDescription.from_json(answer))
    assert_receive {:voice, "slots", empty}
    assert empty == %{}

    {:ok, _} = Den.Voice.Room.join(room, b, "key-b", offer_b)
    assert_receive {:voice, "slots", %{} = mine}
    assert Map.values(mine) == ["key-b"]

    Den.Voice.Room.leave(room, b)
    assert_receive {:voice, "slots", after_leave}
    assert after_leave == %{}

    send(b, :stop)
    PeerConnection.stop(pc_a)
    PeerConnection.stop(pc_b)
  end

  test "a bad offer, a second offer, garbage ICE and a full den are refused; the room goes on" do
    {:ok, room} = Den.Voice.ensure("voice-bad")
    {pc_a, offer_a} = device()
    {:ok, _} = Den.Voice.Room.join(room, self(), "key-a", offer_a)
    assert_receive {:voice, "slots", _}
    other = spawn_link(fn -> receive do: (:stop -> :ok) end)

    # Offers that are not offers, or not this den's kind, are errors, not crashes.
    for bad <- [%{}, %{"type" => "offer"}, %{"type" => "offer", "sdp" => 7}, %{"type" => "offer", "sdp" => "v=0 nonsense"}, %{"type" => "offer", "sdp" => String.duplicate("a", 40_000)}] do
      assert {:error, _} = Den.Voice.Room.join(room, other, "key-x", bad)
    end

    # An offer with too few slots (one audio line).
    {:ok, pc_one} = PeerConnection.start(audio_codecs: [:opus], video_codecs: [])
    {:ok, _} = PeerConnection.add_track(pc_one, MediaStreamTrack.new(:audio))
    {:ok, one} = PeerConnection.create_offer(pc_one)
    assert {:error, :offer} = Den.Voice.Room.join(room, other, "key-x", SessionDescription.to_json(one))
    PeerConnection.stop(pc_one)

    # The same device a second time.
    {_pc, offer_again} = device()
    assert {:error, :already} = Den.Voice.Room.join(room, self(), "key-a", offer_again)

    # ICE that is not ICE does nothing.
    for bad <- [%{"candidate" => 5}, %{"x" => "y"}, %{"candidate" => "candidate:junk"}], do: Den.Voice.Room.ice(room, self(), bad)
    Process.sleep(50)
    assert Process.alive?(room)
    assert Den.Voice.peers() >= 1

    # The den-wide cap: no new connection past it.
    Application.put_env(:den, :voice_max_peers, Den.Voice.peers())
    {_pc, offer_b} = device()
    assert {:error, :busy} = Den.Voice.Room.join(room, other, "key-b", offer_b)
    Application.delete_env(:den, :voice_max_peers)

    # Leaving gives the connection back.
    before = Den.Voice.peers()
    Den.Voice.Room.leave(room, self())
    Process.sleep(50)
    assert Den.Voice.peers() == before - 1
    send(other, :stop)
    PeerConnection.stop(pc_a)
  end

  test "behind 1:1 NAT the den offers its public address for its private one, and only then" do
    on_exit(fn -> Application.delete_env(:den, :public_ip) && Application.delete_env(:den, :voice_public_only) end)
    Application.put_env(:den, :voice_public_only, true)
    refute Den.Voice.ip_ok?({10, 0, 0, 5}), "a private address alone reaches nobody outside"
    assert Den.Voice.srflx({10, 0, 0, 5}) == nil
    Application.put_env(:den, :public_ip, {203, 0, 113, 7})
    assert Den.Voice.ip_ok?({10, 0, 0, 5}), "kept, to carry the public one"
    assert Den.Voice.srflx({10, 0, 0, 5}) == {203, 0, 113, 7}
    assert Den.Voice.srflx({198, 51, 100, 9}) == nil, "a public address needs no stand-in"
    refute Den.Voice.ip_ok?({127, 0, 0, 1})
  end

  test "a device's ICE candidate may not point the den at loopback, link-local or (public only) private addresses" do
    on_exit(fn -> Application.delete_env(:den, :voice_public_only) end)
    c = fn addr -> %{"candidate" => "candidate:1 1 udp 2122260223 #{addr} 54321 typ host", "sdpMid" => "0"} end
    refute Den.Voice.candidate?(c.("127.0.0.1"))
    refute Den.Voice.candidate?(c.("169.254.3.4"))
    refute Den.Voice.candidate?(c.("0.0.0.0"))
    refute Den.Voice.candidate?(c.("::1"))
    assert Den.Voice.candidate?(c.("198.51.100.4"))
    refute Den.Voice.candidate?(c.("2c0f9a3e-7a1b-4d1e-9f00-1b2c3d4e5f60.local")), "no mDNS names"
    assert Den.Voice.candidate?(%{"candidate" => ""})
    assert Den.Voice.candidate?(c.("192.168.50.10"))
    Application.put_env(:den, :voice_public_only, true)
    refute Den.Voice.candidate?(c.("192.168.50.10"))
    refute Den.Voice.candidate?(%{"candidate" => "garbage"})
  end

  test "the den does not listen for mDNS (ex_ice's resolver is stopped at start)" do
    {:ok, sup} = :application.get_supervisor(:ex_ice)
    refute Enum.any?(Supervisor.which_children(sup), fn {id, pid, _, _} -> id == ExICE.Priv.MDNS.Resolver and is_pid(pid) end)
  end

  # A device played by ex_webrtc over a real connection to the den: its own
  # process (the "channel"), relaying ICE both ways, telling the test about
  # slots, the roster and the packets that reach it.
  defp live_device(room, key, test, video \\ false) do
    spawn_link(fn ->
      exts = [%{type: :all, uri: "urn:ietf:params:rtp-hdrext:sdes:mid"}, %{type: :audio, uri: Den.Voice.level_uri()}]
      {:ok, pc} = PeerConnection.start_link(audio_codecs: [:opus], video_codecs: if(video, do: [:vp8], else: []), rtp_header_extensions: exts)
      track = MediaStreamTrack.new(:audio)
      {:ok, _} = PeerConnection.add_track(pc, track)
      [up] = PeerConnection.get_transceivers(pc)
      :ok = PeerConnection.set_transceiver_direction(pc, up.id, :sendonly)
      for _ <- 1..Den.Voice.slots(), do: {:ok, _} = PeerConnection.add_transceiver(pc, :audio, direction: :recvonly)

      # With `video`, a screen up and a stage down, as the page offers them.
      {screen, stage_t} =
        if video do
          vt = MediaStreamTrack.new(:video)
          {:ok, _} = PeerConnection.add_track(pc, vt)
          v = PeerConnection.get_transceivers(pc) |> Enum.find(&(&1.kind == :video))
          :ok = PeerConnection.set_transceiver_direction(pc, v.id, :sendonly)
          {:ok, st} = PeerConnection.add_transceiver(pc, :video, direction: :recvonly)
          {vt.id, st.id}
        else
          {nil, nil}
        end

      {:ok, offer} = PeerConnection.create_offer(pc)
      :ok = PeerConnection.set_local_description(pc, offer)
      [_, level] = Regex.run(~r/a=extmap:(\d+) #{Regex.escape(Den.Voice.level_uri())}/, offer.sdp)
      {:ok, answer} = Den.Voice.Room.join(room, self(), key, SessionDescription.to_json(offer))
      :ok = PeerConnection.set_remote_description(pc, SessionDescription.from_json(answer))
      ts = PeerConnection.get_transceivers(pc)
      mids = for t <- ts, t.direction == :recvonly, t.kind == :audio, into: %{}, do: {t.receiver.track.id, t.mid}
      stage = Enum.find_value(ts, fn t -> t.id == stage_t && t.receiver.track.id end)
      loop(%{pc: pc, room: room, key: key, test: test, track: track.id, level: String.to_integer(level), mids: mids, seq: 0, screen: screen, stage: stage, vseq: 0})
    end)
  end

  # The newest `event` the test has been told for `key`, from its mailbox.
  defp last_of(key, event, acc) do
    receive do
      {^key, ^event, m} -> last_of(key, event, m)
    after
      0 -> acc
    end
  end

  defp loop(d) do
    receive do
      {:voice, "ice", c} ->
        PeerConnection.add_ice_candidate(d.pc, ExWebRTC.ICECandidate.from_json(c))
        loop(d)

      {:voice, event, payload} ->
        send(d.test, {d.key, event, payload})
        loop(d)

      {:ex_webrtc, _, {:ice_candidate, c}} ->
        Den.Voice.Room.ice(d.room, self(), ExWebRTC.ICECandidate.to_json(c))
        loop(d)

      {:ex_webrtc, _, {:connection_state_change, :connected}} ->
        send(d.test, {d.key, :connected})
        loop(d)

      {:ex_webrtc, _, {:rtp, track, _, packet}} when track == d.stage and not is_nil(track) ->
        send(d.test, {d.key, :stage, packet.sequence_number, ExWebRTC.RTP.VP8.keyframe?(packet)})
        loop(d)

      {:ex_webrtc, _, {:rtp, track, _, packet}} ->
        send(d.test, {d.key, :rtp, d.mids[track], packet.sequence_number})
        loop(d)

      {:ex_webrtc, _, {:rtcp, packets}} ->
        for {_, %ExRTCP.Packet.PayloadFeedback.PLI{}} <- packets, do: send(d.test, {d.key, :pli})
        loop(d)

      :pli ->
        PeerConnection.send_pli(d.pc, d.stage)
        loop(d)

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

      {:show_big, n} ->
        for i <- 1..n do
          packet = ExRTP.Packet.new(<<0x10, 0x01, 0, 0>> <> :binary.copy(<<7>>, 1_196), payload_type: 96, sequence_number: d.vseq + i, timestamp: (d.vseq + i) * 3000, ssrc: 2)
          PeerConnection.send_rtp(d.pc, d.screen, packet)
          Process.sleep(5)
        end

        loop(%{d | vseq: d.vseq + n})

      {:speak, n, voice} ->
        for i <- 1..n do
          # ex_webrtc sends no audio level of its own: speech is told by size here.
          packet = ExRTP.Packet.new(:binary.copy(<<0>>, if(voice, do: 160, else: 40)), payload_type: 111, sequence_number: d.seq + i + 1000, timestamp: (d.seq + i) * 960, ssrc: 1)
          PeerConnection.send_rtp(d.pc, d.track, packet)
          Process.sleep(20)
        end

        loop(%{d | seq: d.seq + n})

      :stop ->
        PeerConnection.stop(d.pc)

      _ ->
        loop(d)
    end
  end

  test "more people than slots: each listener hears the most recent speakers, and a slot goes on as one stream" do
    {:ok, room} = Den.Voice.ensure("voice-many")
    keys = for i <- 0..10, do: "key-#{i}"
    devices = Map.new(keys, fn k -> {k, live_device(room, k, self())} end)
    for k <- keys, do: assert_receive({^k, :connected}, 15_000)
    on_exit(fn -> for {_, d} <- devices, do: send(d, :stop) end)

    # Ten others, eight slots: key-0 starts out carrying the first eight to join.
    Process.sleep(300)
    %{keys: roster} = last_of("key-0", "roster", nil)
    slots0 = last_of("key-0", "slots", %{})
    assert Enum.sort(roster) == Enum.sort(keys), "everyone is on the roster"
    assert map_size(slots0) == Den.Voice.slots()
    [outside, _] = (keys -- ["key-0"]) -- Map.values(slots0)
    inside = hd(Map.values(slots0))

    # One in a slot speaks; then one outside speaks, and takes a slot from someone silent.
    send(devices[inside], {:speak, 25, true})
    Process.sleep(600)
    send(devices[outside], {:speak, 40, true})
    assert_receive {"key-0", "slots", slots1}, 3_000
    assert outside in Map.values(slots1), "the newest speaker is carried"
    assert inside in Map.values(slots1), "a recent speaker keeps their slot"
    {mid, _} = Enum.find(slots1, fn {_, k} -> k == outside end)
    # key-0 hears them in that slot, numbered on from what the slot carried before.
    seqs = for _ <- 1..10 do
      assert_receive {"key-0", :rtp, ^mid, seq}, 3_000
      seq
    end
    assert seqs == Enum.sort(seqs) and List.last(seqs) - hd(seqs) < 30, "one stream, its numbers rising: #{inspect(seqs)}"
    for {_, d} <- devices, do: send(d, :stop)
  end

  test "viewers get only the presenter's screen, from a keyframe, as one stream; the presenter gets none" do
    {:ok, room} = Den.Voice.ensure("screen-fwd")
    devs = for k <- ~w(key-a key-b key-c), into: %{}, do: {k, live_device(room, k, self(), true)}
    for k <- Map.keys(devs), do: assert_receive({^k, :connected}, 15_000)
    on_exit(fn -> for {_, d} <- devs, do: send(d, :stop) end)

    :ok = Den.Voice.Room.present(room, devs["key-a"])
    # Not the presenter: dropped.
    send(devs["key-b"], {:show, 5, true})
    # The presenter: delta frames first (no viewer may start on them), then a keyframe and more.
    send(devs["key-a"], {:show, 3, false})
    send(devs["key-a"], {:show, 10, true})

    got = for _ <- 1..10, do: (assert_receive({"key-b", :stage, seq, key?}, 3_000); {seq, key?})
    assert elem(hd(got), 1), "a viewer starts on a keyframe"
    seqs = Enum.map(got, &elem(&1, 0))
    assert seqs == Enum.to_list(hd(seqs)..(hd(seqs) + 9)), "one stream: #{inspect(seqs)}"
    assert_receive {"key-c", :stage, _, true}, 3_000
    refute_received {"key-a", :stage, _, _}
    refute_receive {"key-b", :stage, _, _}, 300
  end

  test "a stream without a keyframe and a late viewer ask the presenter for one; viewers' asks at most once a second" do
    {:ok, room} = Den.Voice.ensure("screen-pli")
    a = live_device(room, "key-a", self(), true)
    on_exit(fn -> send(a, :stop) end)
    assert_receive {"key-a", :connected}, 15_000
    :ok = Den.Voice.Room.present(room, a)
    # The stream starts on delta frames: the den asks for a keyframe.
    send(a, {:show, 3, false})
    assert_receive {"key-a", :pli}, 2_000
    send(a, {:show, 5, true})
    Process.sleep(1_100)
    # A viewer who connects late: asked for at once.
    d = live_device(room, "key-d", self(), true)
    on_exit(fn -> send(d, :stop) end)
    assert_receive {"key-d", :connected}, 15_000
    assert_receive {"key-a", :pli}, 1_000
    send(a, {:show, 5, true})
    assert_receive {"key-d", :stage, _, true}, 3_000
    Process.sleep(1_100)
    # The viewer's decoder asks three times at once: one PLI now, one a second later, no more.
    for _ <- 1..3, do: send(d, :pli)
    assert_receive {"key-a", :pli}, 500
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

  test "a device whose network lets no WebRTC through hears and is heard over its channel" do
    {:ok, room} = Den.Voice.ensure("voice-relay")
    rtc = live_device(room, "key-rtc", self())
    assert_receive {"key-rtc", :connected}, 15_000
    # This test process is the relayed device's channel.
    assert :ok = Den.Voice.Room.join_relay(room, self(), "key-relay")
    assert {:error, :already} = Den.Voice.Room.join_relay(room, self(), "key-relay")
    assert_receive {:voice, "slots", %{"w0" => "key-rtc"}}
    assert_receive {:voice, "roster", %{keys: keys}}
    assert Enum.sort(keys) == ["key-relay", "key-rtc"]
    assert_receive {"key-rtc", "slots", rtc_slots} when map_size(rtc_slots) == 1
    assert Map.values(rtc_slots) == ["key-relay"]

    # WebRTC up, the channel down: the sealed frame as the browser sent it.
    send(rtc, {:speak, 3, true})
    assert_receive {:voice_frame, 0, frame}, 3_000
    assert byte_size(frame) == 160

    # The channel up, WebRTC down: in the relayed device's slot, numbered.
    [{mid, "key-relay"}] = Map.to_list(rtc_slots)
    for _ <- 1..5, do: Den.Voice.Room.frame(room, self(), 30, :binary.copy(<<7>>, 120))
    seqs = for _ <- 1..5 do
      assert_receive {"key-rtc", :rtp, ^mid, seq}, 3_000
      seq
    end
    assert seqs == [0, 1, 2, 3, 4]

    Den.Voice.Room.leave(room, self())
    assert_receive {"key-rtc", "slots", after_leave} when after_leave == %{}, 3_000
    send(rtc, :stop)
  end
end
