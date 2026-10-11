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
  def init(room), do: {:ok, %{room: room, peers: %{}}}

  # A peer: its connection, its own track (once the device sends it), its
  # slots down (mid → track id), and which source each slot carries.
  @impl true
  def handle_call({:join, channel, key, offer}, _from, s) do
    if map_size(s.peers) >= Den.Voice.slots() + 1 do
      {:reply, {:error, :full}, s}
    else
      Process.monitor(channel)
      ports = Application.get_env(:den, :voice_ports, 50_000..50_199)
      {:ok, pc} = PeerConnection.start(ice_port_range: ports, audio_codecs: [:opus], video_codecs: [], controlling_process: self())
      :ok = PeerConnection.set_remote_description(pc, SessionDescription.from_json(offer))
      # The device's first audio transceiver sends to us; the others are its slots.
      [up | downs] = PeerConnection.get_transceivers(pc) |> Enum.filter(&(&1.kind == :audio))
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
      peer = %{pc: pc, key: key, up: nil, slots: slots, carrying: %{}}
      s = %{s | peers: Map.put(s.peers, channel, peer)} |> assign_all()
      {:reply, {:ok, SessionDescription.to_json(answer)}, s}
    end
  end

  @impl true
  def handle_cast({:ice, channel, candidate}, s) do
    with %{pc: pc} <- s.peers[channel], do: PeerConnection.add_ice_candidate(pc, ICECandidate.from_json(candidate))
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
        PeerConnection.stop(p.pc)
        s = assign_all(%{s | peers: rest})
        if map_size(s.peers) == 0, do: send(self(), :stop_if_empty)
        s
    end
  end
end
