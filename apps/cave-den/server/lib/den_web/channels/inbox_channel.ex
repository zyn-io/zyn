defmodule DenWeb.InboxChannel do
  @moduledoc """
  A device's own inbox: only that device may open it. Welcomes to private
  rooms wait here; the device keeps a stock of invitations to itself here.

  * join → waiting Welcomes and how many invitations are left.
  * "publish" {key_packages} → add invitations (the latest 10 are kept).
  * "taken" {ids} → Welcomes the device has joined from.
  """
  # Joins and messages are not logged: a topic names a device or a private room.
  use Phoenix.Channel, log_join: false, log_handle_in: false

  @impl true
  def join("inbox:" <> key, _, socket) do
    DenWeb.Heap.channel()
    if key == socket.assigns.key do
      welcomes = for w <- Den.Inbox.waiting(key), do: %{id: w.id, room: w.room, welcome: Base.encode64(w.bytes)}
      {:ok, %{welcomes: welcomes, key_packages: Den.Inbox.left(key)}, socket}
    else
      {:error, %{reason: "not yours"}}
    end
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

  defp handle("publish", %{"key_packages" => kps}, socket) when is_list(kps) and length(kps) <= 10 do
    bytes = for k <- kps, is_binary(k), byte_size(k) < 6000, {:ok, b} <- [Base.decode64(k)], byte_size(b) < 4096, do: b
    {:reply, {:ok, %{left: Den.Inbox.publish(socket.assigns.key, bytes)}}, socket}
  end

  defp handle("taken", %{"ids" => ids}, socket) when is_list(ids) and length(ids) <= 100 do
    Den.Inbox.taken(socket.assigns.key, Enum.filter(ids, &is_integer/1))
    {:reply, :ok, socket}
  end

  defp handle(_, _, socket), do: {:reply, {:error, %{reason: "unknown"}}, socket}
end
