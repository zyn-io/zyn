defmodule Den.Voice do
  @moduledoc """
  Voice rooms: a selective forwarding unit in Elixir
  (ex_webrtc). Every participant has one WebRTC connection to the den: one
  audio track up, and #{8} audio slots down. Up to `room_max/0` people can
  be in one voice room; each listener's slots carry the eight who spoke most
  recently, by the audio level each browser puts in its packets' headers
  (RFC 6464), and the den tells each device which slot is whose — no
  renegotiation. A slot that changes speaker continues its packet numbers
  and timestamps, so the browser hears one stream.

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

  @doc "Whether members may share their screen in voice (`DEN_SCREEN`, on by default)."
  def screen?, do: Application.get_env(:den, :screen, true)

  @doc "Bits a second a presenter may send (`DEN_SCREEN_BITRATE`, 2.5 Mbps)."
  def screen_bitrate, do: Application.get_env(:den, :screen_bitrate, 2_500_000)

  @doc "How many people one voice room holds (`:voice_room_max`, default 50)."
  def room_max, do: Application.get_env(:den, :voice_room_max, 50)

  @level_uri "urn:ietf:params:rtp-hdrext:ssrc-audio-level"
  def level_uri, do: @level_uri

  @doc """
  Which of the server's addresses voice offers devices: never loopback,
  link-local or a Docker bridge (nobody outside reaches those, and they say
  how the server is set up); with `:voice_public_only`, no private LAN
  address either, for a den on the open internet.
  """
  def ip_ok?({127, _, _, _}), do: false
  def ip_ok?({169, 254, _, _}), do: false
  def ip_ok?({172, 17, _, _}), do: false
  def ip_ok?({0, 0, 0, 0, 0, 0, 0, 1}), do: false
  def ip_ok?({a, _, _, _, _, _, _, _}) when a in 0xFE80..0xFEBF, do: false

  def ip_ok?(ip) do
    not (Application.get_env(:den, :voice_public_only, false) and private?(ip) and is_nil(public_ip()))
  end

  @doc """
  The den's public address when the server sits behind 1:1 NAT (AWS, GCP and
  the like put the public address outside the machine): `DEN_PUBLIC_IP`.
  Each private host address then also goes out as that public address, a
  server-reflexive candidate devices can reach.
  """
  def public_ip, do: Application.get_env(:den, :public_ip)

  def srflx(ip), do: if(public_ip() && private?(ip), do: public_ip(), else: nil)

  defp private?({10, _, _, _}), do: true
  defp private?({172, b, _, _}) when b in 16..31, do: true
  defp private?({192, 168, _, _}), do: true
  defp private?({100, b, _, _}) when b in 64..127, do: true
  defp private?({a, _, _, _, _, _, _, _}) when a in 0xFC00..0xFDFF, do: true
  defp private?(_), do: false
  def peers, do: Registry.count(Den.Voice.Peers)

  @doc "An offer as a device may send it: an SDP offer, of a sane size."
  def offer?(%{"type" => "offer", "sdp" => sdp}) when is_binary(sdp) and byte_size(sdp) in 1..32_768, do: true
  def offer?(_), do: false

  @doc "An ICE candidate as a device may send it."
  def candidate?(%{"candidate" => c} = m) when is_binary(c) and byte_size(c) <= 1024 do
    Enum.all?(Map.keys(m), &(&1 in ["candidate", "sdpMid", "sdpMLineIndex", "usernameFragment"])) and reachable?(c)
  end

  def candidate?(_), do: false

  # A device's candidate names an address the den would send to: never
  # loopback, link-local or unspecified, nor (with :voice_public_only) a
  # private one, so a device cannot point the den at its own network. An
  # empty end-of-candidates line passes; an mDNS name (.local) does not: the
  # den resolves no mDNS on the open internet.
  defp reachable?(""), do: true

  defp reachable?(c) do
    case String.split(c) do
      [_foundation, _component, _proto, _priority, addr | _] ->
        case :inet.parse_address(String.to_charlist(addr)) do
          {:ok, {0, 0, 0, 0}} -> false
          {:ok, {0, 0, 0, 0, 0, 0, 0, 0}} -> false
          {:ok, ip} -> ip_ok?(ip)
          _ -> false
        end

      _ ->
        false
    end
  end

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

  # Who speaks is re-ranked this often (ms); a level at or under this
  # (-dBov, 127 is silence) counts as speaking, as does the voice flag.
  @tick 300
  @speaking 50

  def start_link(room), do: GenServer.start_link(__MODULE__, room, name: {:via, Registry, {Den.Voice.Registry, room}})

  @doc "A device's offer in; the den's answer out. `channel` gets {:voice, event, payload}."
  def join(pid, channel, key, offer), do: GenServer.call(pid, {:join, channel, key, offer}, 15_000)
  def ice(pid, channel, candidate), do: GenServer.cast(pid, {:ice, channel, candidate})
  def leave(pid, channel), do: GenServer.cast(pid, {:leave, channel})

  @doc "A device whose voice goes over its channel instead of WebRTC."
  def join_relay(pid, channel, key), do: GenServer.call(pid, {:join_relay, channel, key}, 15_000)
  def frame(pid, channel, level, frame), do: GenServer.cast(pid, {:frame, channel, level, frame})

  @impl true
  def init(room) do
    # Connections are linked to this room: if it stops, they stop (and give
    # their ports back); if one fails, it is dropped, not the room.
    Process.flag(:trap_exit, true)
    {:ok, %{room: room, peers: %{}, ticking: false}}
  end

  # A peer: its connection, its own track (once the device sends it), its
  # slots down (mid → track id), and which source each slot carries.
  @impl true
  def handle_call({:join, channel, key, offer}, _from, s) do
    # One seat per device: a second join from the same key takes the place
    # of the first.
    s = Enum.reduce(for({c, %{key: ^key}} <- s.peers, c != channel, do: c), s, &drop(&2, &1))

    cond do
      Map.has_key?(s.peers, channel) -> {:reply, {:error, :already}, s}
      not Den.Voice.offer?(offer) -> {:reply, {:error, :offer}, s}
      not sdp?(offer["sdp"]) -> {:reply, {:error, :offer}, s}
      map_size(s.peers) >= Den.Voice.room_max() -> {:reply, {:error, :full}, s}
      Den.Voice.peers() >= Den.Voice.max_peers() -> {:reply, {:error, :busy}, s}
      true -> answer(channel, key, offer, s)
    end
  end

  def handle_call({:join_relay, channel, key}, _from, s) do
    s = Enum.reduce(for({c, %{key: ^key}} <- s.peers, c != channel, do: c), s, &drop(&2, &1))

    cond do
      Map.has_key?(s.peers, channel) -> {:reply, {:error, :already}, s}
      map_size(s.peers) >= Den.Voice.room_max() -> {:reply, {:error, :full}, s}
      Den.Voice.peers() >= Den.Voice.max_peers() -> {:reply, {:error, :busy}, s}
      true ->
        Process.monitor(channel)
        {:ok, _} = Registry.register(Den.Voice.Peers, channel, nil)
        slots = for i <- 0..(Den.Voice.slots() - 1), into: %{}, do: {"w#{i}", i}
        peer = %{kind: :relay, pc: nil, key: key, up: nil, slots: slots, carrying: %{}, out: %{}, connected: true, candidates: 0, level: nil, spoke: nil, joined: System.monotonic_time(:millisecond), seq: 0}
        {:reply, :ok, %{s | peers: Map.put(s.peers, channel, peer)} |> assign_all([channel]) |> roster() |> tick()}
    end
  end

  # A connection for one device: its first audio transceiver sends to us, the
  # next eight are its slots. Anything an offer gets wrong is an error, and
  # the half-made connection is stopped; the room and its peers go on.
  defp answer(channel, key, offer, s) do
    ports = Application.get_env(:den, :voice_ports, 50_000..50_199)

    exts = [%{type: :all, uri: "urn:ietf:params:rtp-hdrext:sdes:mid"}, %{type: :audio, uri: Den.Voice.level_uri()}]

    case PeerConnection.start_link(ice_port_range: ports, ice_ip_filter: &Den.Voice.ip_ok?/1, host_to_srflx_ip_mapper: &Den.Voice.srflx/1, audio_codecs: [:opus], video_codecs: [], rtp_header_extensions: exts, controlling_process: self()) do
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
          now = System.monotonic_time(:millisecond)
          peer = %{kind: :rtc, pc: pc, key: key, up: nil, slots: slots, carrying: %{}, out: %{}, connected: false, candidates: 0, level: level_id(answer.sdp), spoke: nil, joined: now}
          # A connection that never connects gives its port back.
          Process.send_after(self(), {:reap, channel, pc}, Application.get_env(:den, :voice_connect_ms, 20_000))
          s = %{s | peers: Map.put(s.peers, channel, peer)} |> assign_all([channel]) |> roster() |> tick()
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

  # The id the answer gives the audio level extension, if the device offered it.
  defp level_id(sdp) do
    case Regex.run(~r/a=extmap:(\d+)(?:\/\w+)? #{Regex.escape(Den.Voice.level_uri())}/, sdp) do
      [_, id] -> String.to_integer(id)
      _ -> nil
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
         %{pc: pc, candidates: n} when n < 20 <- s.peers[channel] do
      try do
        PeerConnection.add_ice_candidate(pc, ICECandidate.from_json(candidate))
      rescue
        _ -> :ok
      end

      {:noreply, put_in(s.peers[channel].candidates, n + 1)}
    else
      _ -> {:noreply, s}
    end
  end

  def handle_cast({:leave, channel}, s), do: {:noreply, drop(s, channel)}

  # A relayed device's frame: forwarded as the RTP packet it would have been.
  def handle_cast({:frame, channel, level, frame}, s) do
    case s.peers[channel] do
      %{kind: :relay, seq: seq} ->
        now = System.monotonic_time(:millisecond)
        s = put_in(s.peers[channel].seq, seq + 1)
        s = if level <= @speaking, do: put_in(s.peers[channel].spoke, now), else: s
        packet = ExRTP.Packet.new(frame, payload_type: 111, sequence_number: Integer.mod(seq, 65_536), timestamp: Integer.mod(seq * 960, 4_294_967_296), ssrc: 1)
        {:noreply, fan_out(s, channel, packet, now)}

      _ ->
        {:noreply, s}
    end
  end

  @impl true
  def handle_info({:reap, channel, pc}, s) do
    case s.peers[channel] do
      %{pc: ^pc, connected: false} -> {:noreply, drop(s, channel)}
      _ -> {:noreply, s}
    end
  end

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

  # Re-rank who speaks, while there are more people than slots.
  def handle_info(:tick, s), do: {:noreply, %{s | ticking: false} |> assign_all() |> tick()}
  def handle_info(_, s), do: {:noreply, s}

  defp on_pc({:ice_candidate, c}, channel, _peer, s) do
    send(channel, {:voice, "ice", ICECandidate.to_json(c)})
    s
  end

  defp on_pc({:track, %MediaStreamTrack{id: id}}, channel, peer, s) do
    put_in(s.peers[channel], %{peer | up: id})
  end

  # Audio from one device: into its slot on the connection of everyone who
  # carries it now. Its header extensions stay here (their ids are this
  # connection's, not the listener's); the den writes the listener's own.
  defp on_pc({:rtp, id, _rid, packet}, channel, %{up: id} = peer, s) do
    now = System.monotonic_time(:millisecond)
    s = if speaking?(packet, peer.level), do: put_in(s.peers[channel].spoke, now), else: s
    fan_out(s, channel, ExRTP.Packet.remove_extensions(packet), now)
  end

  defp on_pc({:connection_state_change, :failed}, channel, _peer, s), do: drop(s, channel)
  defp on_pc({:connection_state_change, :connected}, channel, peer, s), do: put_in(s.peers[channel], %{peer | connected: true})
  defp on_pc(_, _, _, s), do: s

  # By the level the browser measured; without one, by size: a frame of
  # speech is far larger than one of silence (our frame seal adds 37 bytes).
  defp speaking?(packet, id) do
    with true <- is_integer(id),
         {:ok, ext} <- ExRTP.Packet.fetch_extension(packet, id),
         {:ok, %{voice: voice, level: level}} <- ExRTP.Packet.Extension.AudioLevel.from_raw(ext) do
      voice or level <= @speaking
    else
      _ -> byte_size(packet.payload) > 100
    end
  end

  defp fan_out(s, src, packet, now) do
    Enum.reduce(s.peers, s, fn {other, p}, s ->
      case other != src and p.carrying[src] do
        mid when is_binary(mid) -> forward(s, other, p, mid, src, packet, now)
        _ -> s
      end
    end)
  end

  # To a relayed listener: the frame and its slot's number, over its channel.
  defp forward(s, listener, %{kind: :relay} = p, mid, _src, packet, _now) do
    send(listener, {:voice_frame, p.slots[mid], packet.payload})
    s
  end

  # One packet into a listener's slot. A slot that now carries someone else
  # goes on from its last packet number and timestamp (a marker on the first
  # packet), so the browser takes it as the same stream, not a broken one.
  defp forward(s, listener, p, mid, src, packet, now) do
    o = case p.out[mid] do
      %{src: ^src} = o -> o
      %{last_seq: seq, last_ts: ts, last_at: at} when is_integer(seq) ->
        elapsed = max(now - at, 20)
        %{src: src, seq_off: seq + 1 - packet.sequence_number, ts_off: ts + elapsed * 48 - packet.timestamp, fresh: true}
      _ ->
        %{src: src, seq_off: 0, ts_off: 0, fresh: true}
    end

    seq = Integer.mod(packet.sequence_number + o.seq_off, 65_536)
    ts = Integer.mod(packet.timestamp + o.ts_off, 4_294_967_296)
    PeerConnection.send_rtp(p.pc, p.slots[mid], %{packet | sequence_number: seq, timestamp: ts, marker: packet.marker or o.fresh})
    put_in(s.peers[listener].out[mid], Map.merge(o, %{last_seq: seq, last_ts: ts, last_at: now, fresh: false}))
  end

  # Each listener carries, in its slots, the others who spoke most recently
  # (then the earliest to join); someone keeps their slot while they stay
  # among those. A listener is told only when its slots change (`force`:
  # always, such as on joining).
  defp assign_all(s, force \\ []) do
    peers =
      Map.new(s.peers, fn {listener, p} ->
        ranked =
          for({c, q} <- s.peers, c != listener, do: {c, q})
          # Speakers first, the most recent first (monotonic time may be negative).
          |> Enum.sort_by(fn {_, q} -> {is_nil(q.spoke), -(q.spoke || 0), q.joined} end)
          |> Enum.take(Den.Voice.slots())
          |> Enum.map(&elem(&1, 0))

        kept = Map.filter(p.carrying, fn {src, _} -> src in ranked end)
        free = Map.keys(p.slots) -- Map.values(kept)
        new = Enum.zip(Enum.reject(ranked, &Map.has_key?(kept, &1)), free) |> Map.new()
        {listener, %{p | carrying: Map.merge(kept, new)}}
      end)

    for {listener, p} <- peers, listener in force or p.carrying != s.peers[listener].carrying do
      send(listener, {:voice, "slots", Map.new(p.carrying, fn {src, mid} -> {mid, peers[src].key} end)})
    end

    %{s | peers: peers}
  end

  # Everyone in the voice room, for each device's list (and its keys).
  defp roster(s) do
    keys = for {_, p} <- s.peers, do: p.key
    for {listener, _} <- s.peers, do: send(listener, {:voice, "roster", %{keys: keys}})
    s
  end

  defp tick(%{ticking: false, peers: peers} = s) when map_size(peers) > 9 do
    Process.send_after(self(), :tick, @tick)
    %{s | ticking: true}
  end

  defp tick(s), do: s

  defp drop(s, channel) do
    case Map.pop(s.peers, channel) do
      {nil, _} ->
        s

      {p, rest} ->
        if p.pc do
          Process.unlink(p.pc)
          catch_exit(fn -> PeerConnection.stop(p.pc) end)
        end
        Registry.unregister(Den.Voice.Peers, channel)
        s = assign_all(%{s | peers: rest}) |> roster()
        if map_size(s.peers) == 0, do: send(self(), :stop_if_empty)
        s
    end
  end
end
