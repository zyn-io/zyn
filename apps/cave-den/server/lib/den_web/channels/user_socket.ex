defmodule DenWeb.UserSocket do
  @moduledoc "A device's connection: its session from the door, and nothing else."
  # Connects are not logged: the params carry the session.
  use Phoenix.Socket, log: false

  channel "room:*", DenWeb.RoomChannel
  channel "lobby", DenWeb.LobbyChannel
  channel "inbox:*", DenWeb.InboxChannel
  channel "voice:*", DenWeb.VoiceChannel

  @impl true
  def connect(params, socket, info) do
    cond do
      DenWeb.RateLimit.connects_over?(address(info)) -> :error
      is_binary(session = info[:auth_token] || params["session"]) -> session(session, socket)
      true -> :error
    end
  end

  # Where it comes from, as the door counts addresses (a proxy on this machine
  # says, in X-Forwarded-For).
  defp address(info) do
    ip = get_in(info, [:peer_data, :address]) || {0, 0, 0, 0}
    fwd = for {"x-forwarded-for", v} <- info[:x_headers] || [], do: v
    DenWeb.RateLimit.address(ip, fwd)
  end

  defp session(session, socket) do
    DenWeb.Heap.socket()

    with {:ok, %{key: key}} <- Phoenix.Token.verify(DenWeb.Endpoint, "den", session, max_age: 3 * 86_400),
         %DateTime{} = until <- Den.Rooms.device_until(key),
         :gt <- DateTime.compare(until, DateTime.utc_now()) do
      # A kicked or banned device still connects, to be told so: every
      # channel refuses it ("removed"). A few connections a device, each
      # counted while it lasts (this process is the connection's).
      if length(Registry.lookup(Den.Sockets, key)) >= Den.Limits.get(:sockets) do
        :error
      else
        {:ok, _} = Registry.register(Den.Sockets, key, nil)
        {:ok, assign(socket, :key, key)}
      end
    else
      _ -> :error
    end
  end

  @impl true
  def id(socket), do: "device:" <> socket.assigns.key
end
