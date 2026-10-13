defmodule Den.Voice do
  @moduledoc """
  Voice rooms (DECISIONS §184): a selective forwarding unit in Elixir
  (ex_webrtc). Every participant has one WebRTC connection to the den: one
  audio track up, and #{8} audio slots down. When someone joins, the den
  forwards their audio into a free slot on everyone else's connection and
  tells each device which slot is whose — no renegotiation.

  The audio is end-to-end encrypted before it leaves the browser (each frame
  sealed with AES-GCM under a key every member derives from the room's MLS
  group, new each epoch), so the den forwards frames it cannot hear. It sees
  who is speaking, and how much.
  """

  @slots 8
  def slots, do: @slots

  @doc """
  How many voice connections the whole den holds at once: each takes a UDP
  port from `:voice_ports`, so the cap stays under the range (`:voice_max_peers`,
  default 150), and one client cannot take every port.
  """
  def max_peers, do: Application.get_env(:den, :voice_max_peers, 150)
  def peers, do: Registry.count(Den.Voice.Peers)

  @doc "An offer as a device may send it: an SDP offer, of a sane size."
  def offer?(%{"type" => "offer", "sdp" => sdp}) when is_binary(sdp) and byte_size(sdp) in 1..32_768, do: true
  def offer?(_), do: false

  @doc "An ICE candidate as a device may send it."
  def candidate?(%{"candidate" => c} = m) when is_binary(c) and byte_size(c) <= 1024,
    do: Enum.all?(Map.keys(m), &(&1 in ["candidate", "sdpMid", "sdpMLineIndex", "usernameFragment"]))

  def candidate?(_), do: false

  def ensure(room) do
    case Registry.lookup(Den.Voice.Registry, room) do
      [{pid, _}] -> {:ok, pid}
      [] -> DynamicSupervisor.start_child(Den.Voice.Sup, {Den.Voice.Room, room})
    end
    |> case do
      {:ok, pid} -> {:ok, pid}
      {:error, {:already_started, pid}} -> {:ok, pid}
      other -> other
    end
  end
end

defmodule Den.Voice.Room do
  @moduledoc false
  use GenServer, restart: :temporary
  require Logger
  alias ExWebRTC.{ICECandidate, MediaStreamTrack, PeerConnection, SessionDescription}

  def start_link(room), do: GenServer.start_link(__MODULE__, room, name: {:via, Registry, {Den.Voice.Registry, room}})

  @doc "A device's offer in; the den's answer out. `channel` gets {:voice, event, payload}."
  def join(pid, channel, key, offer), do: GenServer.call(pid, {:join, channel, key, offer}, 15_000)
  def ice(pid, channel, candidate), do: GenServer.cast(pid, {:ice, channel, candidate})
  def leave(pid, channel), do: GenServer.cast(pid, {:leave, channel})

  @impl true
  def init(room) do
    # Connections are linked to this room: if it stops, they stop (and give
    # their ports back); if one fails, it is dropped, not the room.
    Process.flag(:trap_exit, true)
    {:ok, %{room: room, peers: %{}}}
  end

  # A peer: its connection, its own track (once the device sends it), its
  # slots down (mid → track id), and which source each slot carries.
  @impl true
  def handle_call({:join, channel, key, offer}, _from, s) do
    cond do
      Map.has_key?(s.peers, channel) -> {:reply, {:error, :already}, s}
      not Den.Voice.offer?(offer) -> {:reply, {:error, :offer}, s}
      not sdp?(offer["sdp"]) -> {:reply, {:error, :offer}, s}
      map_size(s.peers) >= Den.Voice.slots() + 1 -> {:reply, {:error, :full}, s}
      Den.Voice.peers() >= Den.Voice.max_peers() -> {:reply, {:error, :busy}, s}
      true -> answer(channel, key, offer, s)
    end
  end

  # A connection for one device: its first audio transceiver sends to us, the
  # next eight are its slots. Anything an offer gets wrong is an error, and
  # the half-made connection is stopped; the room and its peers go on.
  defp answer(channel, key, offer, s) do
    ports = Application.get_env(:den, :voice_ports, 50_000..50_199)

    case PeerConnection.start_link(ice_port_range: ports, audio_codecs: [:opus], video_codecs: [], controlling_process: self()) do
      {:ok, pc} ->
        try do
          :ok = PeerConnection.set_remote_description(pc, SessionDescription.from_json(offer))
          [up | downs] = PeerConnection.get_transceivers(pc) |> Enum.filter(&(&1.kind == :audio))
          true = length(downs) >= Den.Voice.slots()
          :ok = PeerConnection.set_transceiver_direction(pc, up.id, :recvonly)

          slots =
            for t <- Enum.take(downs, Den.Voice.slots()), into: %{} do
              track = MediaStreamTrack.new(:audio)
              # ex_webrtc takes a track only on a sending transceiver.
              :ok = PeerConnection.set_transceiver_direction(pc, t.id, :sendonly)
              :ok = PeerConnection.replace_track(pc, t.sender.id, track)
              {t.mid, track.id}
            end

          {:ok, answer} = PeerConnection.create_answer(pc)
          :ok = PeerConnection.set_local_description(pc, answer)
          Process.monitor(channel)
          {:ok, _} = Registry.register(Den.Voice.Peers, channel, nil)
          peer = %{pc: pc, key: key, up: nil, slots: slots, carrying: %{}}
          s = %{s | peers: Map.put(s.peers, channel, peer)} |> assign_all()
          {:reply, {:ok, SessionDescription.to_json(answer)}, s}
        rescue
          _ -> stop_pc(pc, s, :offer)
        catch
          _, _ -> stop_pc(pc, s, :offer)
        end

      _ ->
        {:reply, {:error, :busy}, s}
    end
  end

  # ExSDP raises on some junk rather than answering an error.
  defp sdp?(sdp) do
    match?({:ok, _}, ExSDP.parse(sdp))
  rescue
    _ -> false
  end

  defp stop_pc(pc, s, why) do
    Process.unlink(pc)
    # It may have ended already (a bad offer can end it).
    catch_exit(fn -> PeerConnection.stop(pc) end)
    {:reply, {:error, why}, s}
  end

  defp catch_exit(f) do
    f.()
  catch
    :exit, _ -> :ok
  end

  @impl true
  def handle_cast({:ice, channel, candidate}, s) do
    with true <- Den.Voice.candidate?(candidate),
         %{pc: pc} <- s.peers[channel] do
      try do
        PeerConnection.add_ice_candidate(pc, ICECandidate.from_json(candidate))
      rescue
        _ -> :ok
      end
    end

    {:noreply, s}
  end

  def handle_cast({:leave, channel}, s), do: {:noreply, drop(s, channel)}

  @impl true
  def handle_info({:ex_webrtc, pc, msg}, s) do
    case Enum.find(s.peers, fn {_, p} -> p.pc == pc end) do
      nil -> {:noreply, s}
      {channel, peer} -> {:noreply, on_pc(msg, channel, peer, s)}
    end
  end

  def handle_info({:DOWN, _, :process, channel, _}, s), do: {:noreply, drop(s, channel)}

  # A connection that ended on its own: drop its peer, keep the room.
  def handle_info({:EXIT, pc, _}, s) do
    case Enum.find(s.peers, fn {_, p} -> p.pc == pc end) do
      {channel, _} -> {:noreply, drop(s, channel)}
      nil -> {:noreply, s}
    end
  end
  def handle_info(:stop_if_empty, %{peers: peers} = s) when map_size(peers) == 0, do: {:stop, :normal, s}
  def handle_info(_, s), do: {:noreply, s}

  defp on_pc({:ice_candidate, c}, channel, _peer, s) do
    send(channel, {:voice, "ice", ICECandidate.to_json(c)})
    s
  end

  defp on_pc({:track, %MediaStreamTrack{id: id}}, channel, peer, s) do
    put_in(s.peers[channel], %{peer | up: id})
  end

  # Audio from one device: into its slot on everyone else's connection.
  defp on_pc({:rtp, id, _rid, packet}, channel, %{up: id}, s) do
    for {other, p} <- s.peers, other != channel, mid = p.carrying[channel], mid != nil do
      PeerConnection.send_rtp(p.pc, p.slots[mid], packet)
    end

    s
  end

  defp on_pc({:connection_state_change, :failed}, channel, _peer, s), do: drop(s, channel)
  defp on_pc(_, _, _, s), do: s

  # Every listener carries every other speaker in a slot of its own.
  defp assign_all(s) do
    peers =
      Map.new(s.peers, fn {listener, p} ->
        wanted = Map.keys(s.peers) -- [listener]
        kept = Map.filter(p.carrying, fn {src, _} -> src in wanted end)
        free = Map.keys(p.slots) -- Map.values(kept)
        new = Enum.zip(wanted -- Map.keys(kept), free) |> Map.new()
        {listener, %{p | carrying: Map.merge(kept, new)}}
      end)

    for {listener, p} <- peers do
      send(listener, {:voice, "slots", Map.new(p.carrying, fn {src, mid} -> {mid, peers[src].key} end)})
    end

    %{s | peers: peers}
  end

  defp drop(s, channel) do
    case Map.pop(s.peers, channel) do
      {nil, _} ->
        s

      {p, rest} ->
        Process.unlink(p.pc)
        catch_exit(fn -> PeerConnection.stop(p.pc) end)
        Registry.unregister(Den.Voice.Peers, channel)
        s = assign_all(%{s | peers: rest})
        if map_size(s.peers) == 0, do: send(self(), :stop_if_empty)
        s
    end
  end
end
