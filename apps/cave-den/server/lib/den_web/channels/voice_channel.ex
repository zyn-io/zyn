defmodule DenWeb.VoiceChannel do
  @moduledoc """
  Signalling for a voice room: the device's offer and ICE candidates in, the
  den's answer, candidates and slot map ({mid → speaker's device key}) out.
  The media itself goes over WebRTC, sealed end to end by the devices; or,
  for a device whose network lets no WebRTC through ("relay_join"), over this
  channel: each sealed frame up as `frame` (one level byte, then the frame),
  and down as `f` (its slot's number, then the frame).
  """
  # Joins and messages are not logged: a topic names a device or a private room.
  use Phoenix.Channel, log_join: false, log_handle_in: false

  @impl true
  def join("voice:" <> room, _, socket) do
    DenWeb.Heap.channel()
    case Den.Rooms.get(room) do
      nil -> {:error, %{reason: "no such room"}}
      r ->
        cond do
          not Den.Rooms.may_enter?(r, socket.assigns.key) -> {:error, %{reason: "no such room"}}
          Den.Mod.muted?(socket.assigns.key) -> {:error, %{reason: "You are timed out and cannot join voice for now."}}
          true -> {:ok, %{slots: Den.Voice.slots()}, assign(socket, :voice, room)}
        end
    end
  end

  # A private room's commit took this device out: it leaves voice too.
  intercept ["members"]

  @impl true
  def handle_out("members", %{keys: keys}, socket) do
    if socket.assigns.key in keys, do: {:noreply, socket}, else: {:stop, {:shutdown, :removed}, socket}
  end

  # Every event, at a rate the den can answer (DenWeb.Throttle).
  @impl true
  def handle_in(event, payload, socket) do
    cond do
      DenWeb.Throttle.allow?(event) -> handle(event, payload, socket)
      event == "typing" -> {:noreply, socket}
      true -> {:reply, DenWeb.Throttle.refused(), socket}
    end
  end

  defp handle("offer", %{"type" => "offer"} = offer, socket) do
    with {:ok, pid} <- Den.Voice.ensure(socket.assigns.voice),
         {:ok, answer} <- Den.Voice.Room.join(pid, self(), socket.assigns.key, offer) do
      {:reply, {:ok, answer}, assign(socket, :room_pid, pid)}
    else
      {:error, :full} -> {:reply, {:error, %{reason: "the voice room is full (#{Den.Voice.room_max()})"}}, socket}
      {:error, :busy} -> {:reply, {:error, %{reason: "voice is busy on this den; try again shortly"}}, socket}
      {:error, :already} -> {:reply, {:error, %{reason: "this device is already in the voice room"}}, socket}
      {:error, :offer} -> {:reply, {:error, %{reason: "that is not a voice offer this den can answer"}}, socket}
      _ -> {:reply, {:error, %{reason: "could not join the voice room"}}, socket}
    end
  end

  defp handle("offer", _, socket), do: {:reply, {:error, %{reason: "that is not a voice offer this den can answer"}}, socket}

  defp handle("ice", candidate, socket) when is_map(candidate) do
    if pid = socket.assigns[:room_pid], do: Den.Voice.Room.ice(pid, self(), candidate)
    {:noreply, socket}
  end

  defp handle("relay_join", _, socket) do
    with {:ok, pid} <- Den.Voice.ensure(socket.assigns.voice),
         :ok <- Den.Voice.Room.join_relay(pid, self(), socket.assigns.key) do
      {:reply, {:ok, %{slots: Den.Voice.slots()}}, assign(socket, :room_pid, pid)}
    else
      {:error, :full} -> {:reply, {:error, %{reason: "the voice room is full (#{Den.Voice.room_max()})"}}, socket}
      {:error, :busy} -> {:reply, {:error, %{reason: "voice is busy on this den; try again shortly"}}, socket}
      {:error, :already} -> {:reply, {:error, %{reason: "this device is already in the voice room"}}, socket}
      _ -> {:reply, {:error, %{reason: "could not join the voice room"}}, socket}
    end
  end

  # One sealed frame (at most a packet's worth) and its audio level byte.
  defp handle("frame", {:binary, <<level, frame::binary>>}, socket) when byte_size(frame) in 38..1400 do
    if pid = socket.assigns[:room_pid], do: Den.Voice.Room.frame(pid, self(), level, frame)
    {:noreply, socket}
  end

  defp handle("frame", _, socket), do: {:noreply, socket}

  defp handle("leave", _, socket) do
    if pid = socket.assigns[:room_pid], do: Den.Voice.Room.leave(pid, self())
    {:reply, :ok, assign(socket, :room_pid, nil)}
  end

  defp handle(_, _, socket), do: {:reply, {:error, %{reason: "unknown"}}, socket}

  @impl true
  def handle_info({:voice_frame, slot, frame}, socket) do
    push(socket, "f", {:binary, <<slot, frame::binary>>})
    {:noreply, socket}
  end

  def handle_info({:voice, event, payload}, socket) do
    push(socket, event, payload)
    {:noreply, socket}
  end

  @impl true
  def terminate(_, socket) do
    if pid = socket.assigns[:room_pid], do: Den.Voice.Room.leave(pid, self())
    :ok
  end
end
