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
end
