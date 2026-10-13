defmodule Den.Inbox do
  @moduledoc """
  Private rooms grow by invitation (MLS Add + Welcome), not by GroupInfo. A
  device publishes a few one-time invitations to itself (KeyPackages); whoever
  adds it claims one, and the Welcome waits in the device's inbox until it
  takes it. The den sees which device invited which, as it sees who is in
  which room; it reads none of it.
  """
  import Ecto.Query
  alias Den.Repo

  @keep 10

  @doc "Keep a device's latest #{10} invitations."
  def publish(key, kps) when is_list(kps) do
    now = Den.Rooms.now()
    Repo.insert_all("key_packages", Enum.map(kps, &%{key: key, bytes: &1, inserted_at: now}))
    ids = Repo.all(from k in "key_packages", where: k.key == ^key, order_by: [desc: k.id], select: k.id) |> Enum.drop(@keep)
    Repo.delete_all(from k in "key_packages", where: k.id in ^ids)
    left(key)
  end

  def left(key), do: Repo.one(from k in "key_packages", where: k.key == ^key, select: count(k.id))

  @doc "One invitation to `key`, used up."
  def claim(key) do
    Repo.transaction(fn ->
      case Repo.one(from k in "key_packages", where: k.key == ^key, order_by: k.id, limit: 1, select: {k.id, k.bytes}) do
        nil ->
          Repo.rollback(:none)

        {id, bytes} ->
          Repo.delete_all(from k in "key_packages", where: k.id == ^id)
          bytes
      end
    end)
  end

  @doc "A Welcome for each device with room in its inbox (`:inbox` waiting at most), kept until it takes it."
  def deliver(keys, room, welcome) do
    now = Den.Rooms.now()
    cap = Den.Limits.get(:inbox)
    keys = Enum.filter(Enum.uniq(keys), &(Repo.one(from i in "inbox", where: i.key == ^&1, select: count(i.id)) < cap))
    Repo.insert_all("inbox", Enum.map(keys, &%{key: &1, room: room, bytes: welcome, inserted_at: now}))
    for k <- keys, do: DenWeb.Endpoint.broadcast("inbox:" <> k, "welcome", %{room: room, welcome: Base.encode64(welcome)})
    :ok
  end

  def waiting(key), do: Repo.all(from i in "inbox", where: i.key == ^key, order_by: i.id, select: %{id: i.id, room: i.room, bytes: i.bytes})

  def taken(key, ids), do: Repo.delete_all(from i in "inbox", where: i.key == ^key and i.id in ^ids)
end
