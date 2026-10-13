defmodule Den.VoiceTest do
  # The forwarding unit's side of a join, with ex_webrtc playing the browser:
  # one track up, eight slots down, an answer that sends on every slot, and a
  # slot map that names each other speaker.
  use ExUnit.Case, async: false
  alias ExWebRTC.{MediaStreamTrack, PeerConnection, SessionDescription}

  defp device do
    {:ok, pc} = PeerConnection.start(audio_codecs: [:opus], video_codecs: [])
    {:ok, _} = PeerConnection.add_track(pc, MediaStreamTrack.new(:audio))
    [up] = PeerConnection.get_transceivers(pc)
    :ok = PeerConnection.set_transceiver_direction(pc, up.id, :sendonly)
    for _ <- 1..Den.Voice.slots(), do: {:ok, _} = PeerConnection.add_transceiver(pc, :audio, direction: :recvonly)
    {:ok, offer} = PeerConnection.create_offer(pc)
    :ok = PeerConnection.set_local_description(pc, offer)
    {pc, SessionDescription.to_json(offer)}
  end

  defp directions(sdp), do: Regex.scan(~r/a=(sendonly|recvonly|sendrecv|inactive)/, sdp, capture: :all_but_first) |> List.flatten()

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
end
