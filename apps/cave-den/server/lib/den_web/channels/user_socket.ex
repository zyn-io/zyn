defmodule DenWeb.UserSocket do
  @moduledoc "A device's connection: its session from the door, and nothing else."
  # Connects are not logged: the params carry the session.
  use Phoenix.Socket, log: false

  channel "room:*", DenWeb.RoomChannel
  channel "lobby", DenWeb.LobbyChannel
  channel "inbox:*", DenWeb.InboxChannel
  channel "voice:*", DenWeb.VoiceChannel

  @impl true
  def connect(%{"session" => session}, socket, _info) do
    DenWeb.Heap.socket()

    with {:ok, %{key: key}} <- Phoenix.Token.verify(DenWeb.Endpoint, "den", session, max_age: 3 * 86_400),
         %DateTime{} = until <- Den.Rooms.device_until(key),
         :gt <- DateTime.compare(until, DateTime.utc_now()) do
      {:ok, assign(socket, :key, key)}
    else
      _ -> :error
    end
  end

  def connect(_, _, _), do: :error

  @impl true
  def id(socket), do: "device:" <> socket.assigns.key
end
