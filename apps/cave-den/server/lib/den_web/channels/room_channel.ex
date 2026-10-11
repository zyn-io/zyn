defmodule DenWeb.RoomChannel do
  @moduledoc """
  One room, for one device. Everything that crosses it is MLS ciphertext or a
  GroupInfo; the den orders and stores, it does not read.

  * join → the room's epoch and sequence, its latest GroupInfo (to join by
    external commit), and the device keys still admitted in it (members'
    devices take out anyone else).
  * "state" → the same as join's answer, again (after a refused join).
  * "history" {after} → ciphertext after a sequence number.
  * "create" {group_info} → the room's first group, if it has none.
  * "relay" {bytes} → a commit (only at the room's epoch: the first wins) or
    words; relayed to the others, stored for `retention_days`.
  * "info" {epoch, group_info} → the GroupInfo for the epoch the room is in.
  """
  use Phoenix.Channel
  alias Den.Rooms

  @max 65_536

  @impl true
  def join("room:" <> id, _params, socket) do
    case Rooms.get(id) do
      nil ->
        {:error, %{reason: "no such room"}}

      room ->
        Rooms.seat(id, socket.assigns.key)
        info = room.group_info && Base.encode64(room.group_info)
        {:ok, %{title: room.title, epoch: room.epoch, seq: room.seq, group_info: info, present: Rooms.present(id)}, assign(socket, :room, id)}
    end
  end

  @impl true
  def handle_in("history", %{"after" => n}, socket) when is_integer(n) do
    msgs = for m <- Rooms.since(socket.assigns.room, n), do: %{seq: m.seq, epoch: m.epoch, commit: m.commit, bytes: Base.encode64(m.bytes)}
    {:reply, {:ok, %{messages: msgs}}, socket}
  end

  def handle_in("state", _, socket) do
    room = Rooms.get(socket.assigns.room)
    info = room.group_info && Base.encode64(room.group_info)
    {:reply, {:ok, %{epoch: room.epoch, seq: room.seq, group_info: info, present: Rooms.present(room.id)}}, socket}
  end

  def handle_in("create", %{"group_info" => b64}, socket) do
    with {:ok, info} <- decode(b64),
         :ok <- Rooms.create(socket.assigns.room, info) do
      broadcast_from!(socket, "info", %{epoch: 0, group_info: b64})
      {:reply, :ok, socket}
    else
      {:error, why} -> {:reply, {:error, %{reason: to_string(why)}}, socket}
    end
  end

  def handle_in("info", %{"epoch" => epoch, "group_info" => b64}, socket) when is_integer(epoch) do
    with {:ok, info} <- decode(b64),
         :ok <- Rooms.info(socket.assigns.room, epoch, info) do
      {:reply, :ok, socket}
    else
      {:error, why} -> {:reply, {:error, %{reason: to_string(why)}}, socket}
    end
  end

  def handle_in("relay", %{"bytes" => b64}, socket) do
    with {:ok, bytes} <- decode(b64),
         {:ok, m} <- Rooms.relay(socket.assigns.room, bytes) do
      broadcast_from!(socket, "msg", %{seq: m.seq, epoch: m.epoch, commit: m.commit, bytes: b64})
      {:reply, {:ok, %{seq: m.seq, epoch: if(m.commit, do: m.epoch + 1, else: m.epoch)}}, socket}
    else
      {:error, {:epoch, e}} -> {:reply, {:error, %{reason: "epoch", epoch: e}}, socket}
      {:error, {:no_group, _}} -> {:reply, {:error, %{reason: "no group yet"}}, socket}
      {:error, why} -> {:reply, {:error, %{reason: inspect(why)}}, socket}
    end
  end

  def handle_in(_, _, socket), do: {:reply, {:error, %{reason: "unknown"}}, socket}

  defp decode(b64) when is_binary(b64) and byte_size(b64) <= @max * 2 do
    case Base.decode64(b64) do
      {:ok, b} when byte_size(b) <= @max -> {:ok, b}
      _ -> {:error, :bytes}
    end
  end

  defp decode(_), do: {:error, :bytes}
end
