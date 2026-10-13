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
  # Joins and messages are not logged: a topic names a device or a private room.
  use Phoenix.Channel, log_join: false, log_handle_in: false
  alias Den.Rooms

  @max 65_536

  @impl true
  def join("room:" <> id, _params, socket) do
    DenWeb.Heap.channel()
    case Rooms.get(id) do
      nil ->
        {:error, %{reason: "no such room"}}

      room ->
        if Rooms.may_enter?(room, socket.assigns.key), do: enter(id, room, socket), else: {:error, %{reason: "no such room"}}
    end
  end

  defp enter(id, room, socket) do
        Rooms.seat(id, socket.assigns.key)
        Registry.register(Den.Online, id, socket.assigns.key)
        info = room.group_info && Base.encode64(room.group_info)
        {:ok, %{title: room.title, kind: room.kind, epoch: room.epoch, seq: room.seq, group_info: info, info_epoch: room.info_epoch, present: Rooms.present(id)}, socket |> assign(:room, id) |> assign(:kind, room.kind)}
  end

  # After a commit in a private room: a device it took out leaves the channel
  # at once, with no more words, presence or history from it.
  intercept ["members"]

  @impl true
  def handle_out("members", %{keys: keys}, socket) do
    if socket.assigns.key in keys, do: {:noreply, socket}, else: {:stop, {:shutdown, :removed}, socket}
  end

  @impl true
  # At most `:history_bytes` of ciphertext a reply (and always one message);
  # `more` says to ask again after the last.
  def handle_in("history", %{"after" => n}, socket) when is_integer(n) do
    rows = Rooms.since(socket.assigns.room, n)
    cap = Den.Limits.get(:history_bytes)

    {kept, _} =
      Enum.reduce_while(rows, {[], 0}, fn m, {acc, size} ->
        size = size + byte_size(m.bytes)
        if acc != [] and size > cap, do: {:halt, {acc, size}}, else: {:cont, {[m | acc], size}}
      end)

    kept = Enum.reverse(kept)
    msgs = for m <- kept, do: %{seq: m.seq, epoch: m.epoch, commit: m.commit, bytes: Base.encode64(m.bytes)}
    {:reply, {:ok, %{messages: msgs, more: length(kept) < length(rows) or length(rows) == 500}}, socket}
  end

  def handle_in("state", _, socket) do
    room = Rooms.get(socket.assigns.room)
    info = room.group_info && Base.encode64(room.group_info)
    {:reply, {:ok, %{epoch: room.epoch, seq: room.seq, group_info: info, info_epoch: room.info_epoch, present: Rooms.present(room.id)}}, socket}
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

  # This device cannot apply the room's last commit: its signed report. When
  # most members report it, the den undoes the commit and tells the room.
  def handle_in("report", %{"seq" => seq, "sig" => sig}, socket) when is_integer(seq) do
    case Rooms.report(socket.assigns.room, seq, socket.assigns.key, sig) do
      {:ok, :rolled_back, epoch, to} ->
        broadcast!(socket, "rolled_back", %{epoch: epoch, seq: to})
        {:reply, {:ok, %{rolled_back: true}}, socket}

      {:ok, :noted} ->
        {:reply, {:ok, %{rolled_back: false}}, socket}

      {:error, why} ->
        {:reply, {:error, %{reason: to_string(why)}}, socket}
    end
  end

  def handle_in("relay", %{"bytes" => b64}, socket) do
    with {:ok, bytes} <- decode(b64),
         true <- Den.Limits.disk_ok?() || {:error, :disk},
         :ok <- Rooms.quota(socket.assigns.key, "message_bytes", Den.Limits.get(:message_bytes), byte_size(bytes)),
         {:ok, m} <- Rooms.relay(socket.assigns.room, bytes, socket.assigns.key) do
      broadcast_from!(socket, "msg", %{seq: m.seq, epoch: m.epoch, commit: m.commit, bytes: b64})
      if m.commit and socket.assigns.kind == "private", do: Rooms.announce_members(socket.assigns.room)
      {:reply, {:ok, %{seq: m.seq, epoch: if(m.commit, do: m.epoch + 1, else: m.epoch)}}, socket}
    else
      {:error, {:epoch, e}} -> {:reply, {:error, %{reason: "epoch", epoch: e}}, socket}
      {:error, {:no_group, _}} -> {:reply, {:error, %{reason: "no group yet"}}, socket}
      {:error, :private} -> {:reply, {:error, %{reason: "a private room is joined by invitation"}}, socket}
      {:error, :not_yours} -> {:reply, {:error, %{reason: "a commit is signed by the device that sends it"}}, socket}
      {:error, :not_member} -> {:reply, {:error, %{reason: "this device is not a member of the room"}}, socket}
      {:error, :barred} -> {:reply, {:error, %{reason: "most members could not apply this device's last change here; it may not change the room until tomorrow"}}, socket}
      {:error, :quota} -> {:reply, {:error, %{reason: "this device has written enough for today"}}, socket}
      {:error, :disk} -> {:reply, {:error, %{reason: "the den is full for now; try again later"}}, socket}
      {:error, {:bad_commit, _}} -> {:reply, {:error, %{reason: "that commit does not check out against the room"}}, socket}
      {:error, why} -> {:reply, {:error, %{reason: inspect(why)}}, socket}
    end
  end

  # Someone is writing: their device name only, not stored.
  def handle_in("typing", _, socket) do
    broadcast_from!(socket, "typing", %{key: socket.assigns.key})
    {:noreply, socket}
  end

  # After an Add the den ordered: the Welcome for the devices added.
  def handle_in("invite", %{"to" => keys, "welcome" => b64}, socket) when is_list(keys) and length(keys) <= 50 do
    with {:ok, welcome} <- decode(b64),
         true <- Enum.all?(keys, &(is_binary(&1) and &1 =~ ~r/\A[0-9a-f]{64}\z/)) || {:error, :keys},
         true <- Den.Limits.disk_ok?() || {:error, :disk},
         # Only devices the den has admitted get a Welcome; each copy is
         # counted against the sender's daily bytes.
         to = Den.Inbox.known(keys),
         :ok <- Rooms.quota(socket.assigns.key, "invite", Den.Limits.get(:invites)),
         :ok <- Rooms.quota(socket.assigns.key, "message_bytes", Den.Limits.get(:message_bytes), byte_size(welcome) * length(to)) do
      Den.Inbox.deliver(to, socket.assigns.room, welcome)
      {:reply, :ok, socket}
    else
      {:error, why} -> {:reply, {:error, %{reason: to_string(why)}}, socket}
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
