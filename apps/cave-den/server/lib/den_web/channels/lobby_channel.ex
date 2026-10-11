defmodule DenWeb.LobbyChannel do
  @moduledoc """
  What every admitted device shares: the list of open and proposal rooms, new
  rooms as they are made, and the means to start a private one or invite a
  device to it.

  * join → the rooms.
  * "open_room" {title} → a room any member can join (3 a day per device).
  * "private_room" {id} → reserve an unguessable private room id.
  * "claim" {key} → one of that device's invitations (KeyPackages).
  """
  use Phoenix.Channel
  alias Den.Rooms

  @impl true
  def join("lobby", _, socket), do: {:ok, %{rooms: Rooms.listed()}, socket}

  @impl true
  def handle_in("open_room", %{"title" => title}, socket) when is_binary(title) do
    with :ok <- Rooms.quota(socket.assigns.key, "room", 3),
         {:ok, r} <- Rooms.open_room(title) do
      broadcast!(socket, "room", %{id: r.id, title: r.title, kind: r.kind})
      {:reply, {:ok, %{id: r.id, title: r.title}}, socket}
    else
      {:error, why} -> {:reply, {:error, %{reason: said(why, "room")}}, socket}
    end
  end

  def handle_in("private_room", %{"id" => id}, socket) when is_binary(id) do
    with :ok <- Rooms.quota(socket.assigns.key, "private", 50),
         {:ok, r} <- Rooms.private_room(id) do
      {:reply, {:ok, %{id: r.id}}, socket}
    else
      {:error, why} -> {:reply, {:error, %{reason: said(why, "private")}}, socket}
    end
  end

  def handle_in("claim", %{"key" => key}, socket) when is_binary(key) do
    with :ok <- Rooms.quota(socket.assigns.key, "claim", 100),
         {:ok, bytes} <- Den.Inbox.claim(key) do
      {:reply, {:ok, %{key_package: Base.encode64(bytes)}}, socket}
    else
      {:error, :none} -> {:reply, {:error, %{reason: "That member's device has to come online once before it can be invited."}}, socket}
      {:error, why} -> {:reply, {:error, %{reason: said(why, "claim")}}, socket}
    end
  end

  def handle_in(_, _, socket), do: {:reply, {:error, %{reason: "unknown"}}, socket}

  # What a member reads when the den says no.
  defp said(:quota, "room"), do: "A device can open 3 rooms a day. Try again tomorrow."
  defp said(:quota, "private"), do: "A device can start 50 private rooms a day. Try again tomorrow."
  defp said(:quota, "claim"), do: "That is enough invitations for today. Try again tomorrow."
  defp said(:title, _), do: "Give the room a name."
  defp said(why, _), do: to_string(why)
end
