defmodule DenWeb.VoiceChannel do
  @moduledoc """
  Signalling for a voice room: the device's offer and ICE candidates in, the
  den's answer, candidates and slot map ({mid → speaker's device key}) out.
  The media itself goes over WebRTC, sealed end to end by the devices.
  """
  # Joins and messages are not logged: a topic names a device or a private room.
  use Phoenix.Channel, log_join: false, log_handle_in: false

  @impl true
  def join("voice:" <> room, _, socket) do
    DenWeb.Heap.channel()
    case Den.Rooms.get(room) do
      nil -> {:error, %{reason: "no such room"}}
      r -> if Den.Rooms.may_enter?(r, socket.assigns.key), do: {:ok, %{slots: Den.Voice.slots()}, assign(socket, :voice, room)}, else: {:error, %{reason: "no such room"}}
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
      {:error, :full} -> {:reply, {:error, %{reason: "the voice room is full (#{Den.Voice.slots() + 1})"}}, socket}
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

  defp handle("leave", _, socket) do
    if pid = socket.assigns[:room_pid], do: Den.Voice.Room.leave(pid, self())
    {:reply, :ok, assign(socket, :room_pid, nil)}
  end

  defp handle(_, _, socket), do: {:reply, {:error, %{reason: "unknown"}}, socket}

  @impl true
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
