defmodule Den.Rooms do
  @moduledoc """
  The den's state, and the only rules it applies to rooms: one commit per
  epoch (the first to arrive), messages for the epoch a room is in (or a few
  before), and ciphertext kept for `retention_days`. It never holds a key.
  """
  import Ecto.Query
  alias Den.Repo

  @past_epochs 3

  defmodule Room do
    use Ecto.Schema
    @primary_key {:id, :string, autogenerate: false}
    schema "rooms" do
      field :title, :string
      field :epoch, :integer, default: 0
      field :group_info, :binary
      field :seq, :integer, default: 0
      timestamps(type: :utc_datetime)
    end
  end

  defmodule Message do
    use Ecto.Schema
    schema "messages" do
      field :room, :string
      field :seq, :integer
      field :epoch, :integer
      field :commit, :boolean
      field :bytes, :binary
      timestamps(type: :utc_datetime, updated_at: false)
    end
  end

  defmodule Device do
    use Ecto.Schema
    @primary_key {:key, :string, autogenerate: false}
    schema "devices" do
      field :until, :utc_datetime
      timestamps(type: :utc_datetime)
    end
  end

  defmodule RoomDevice do
    use Ecto.Schema
    @primary_key false
    schema "room_devices" do
      field :room, :string, primary_key: true
      field :key, :string, primary_key: true
      timestamps(type: :utc_datetime)
    end
  end

  @doc "Make the configured rooms if they are not there."
  def seed do
    for {id, title} <- Application.fetch_env!(:den, :rooms) do
      Repo.insert!(%Room{id: id, title: title}, on_conflict: [set: [title: title]], conflict_target: :id)
    end

    :ok
  end

  def get(id), do: Repo.get(Room, id)
  def all, do: Repo.all(from r in Room, order_by: r.id)

  def now, do: DateTime.utc_now() |> DateTime.truncate(:second)

  @doc "A device is in a room: recorded so members can take it out when its day ends."
  def seat(room, key) do
    Repo.insert!(%RoomDevice{room: room, key: key}, on_conflict: :nothing, conflict_target: [:room, :key])
    :ok
  end

  @doc "Device keys in a room whose day has not ended."
  def present(room) do
    now = now()
    Repo.all(from rd in RoomDevice, join: d in Device, on: d.key == rd.key, where: rd.room == ^room and d.until > ^now, select: rd.key)
  end

  @doc "A room's first GroupInfo, from whoever makes the group first."
  def create(room, group_info) do
    {n, _} = Repo.update_all(from(r in Room, where: r.id == ^room and is_nil(r.group_info)), set: [group_info: group_info, epoch: 0, updated_at: now()])
    if n == 1, do: :ok, else: {:error, :exists}
  end

  @doc "The GroupInfo for the epoch the room is in, from a member who is in it."
  def info(room, epoch, group_info) do
    {n, _} = Repo.update_all(from(r in Room, where: r.id == ^room and r.epoch == ^epoch), set: [group_info: group_info, updated_at: now()])
    if n == 1, do: :ok, else: {:error, :stale}
  end

  @doc """
  Relay `bytes` in `room`: a commit only at the room's epoch (it moves the room
  to the next), words at that epoch or a few before. Returns the message.
  """
  def relay(room, bytes) do
    with {:ok, env} <- Den.Mls.envelope(bytes),
         true <- env.room == room || {:error, :wrong_room} do
      Repo.transaction(fn ->
        r = Repo.get!(Room, room)

        cond do
          is_nil(r.group_info) ->
            Repo.rollback({:no_group, r.epoch})

          env.commit and env.epoch != r.epoch ->
            Repo.rollback({:epoch, r.epoch})

          not env.commit and (env.epoch > r.epoch or env.epoch < r.epoch - @past_epochs) ->
            Repo.rollback({:epoch, r.epoch})

          true ->
            seq = r.seq + 1
            epoch = if env.commit, do: r.epoch + 1, else: r.epoch
            # Optimistic: the room must still be where we read it.
            {1, _} = Repo.update_all(from(x in Room, where: x.id == ^room and x.seq == ^r.seq), set: [seq: seq, epoch: epoch, updated_at: now()])
            Repo.insert!(%Message{room: room, seq: seq, epoch: env.epoch, commit: env.commit, bytes: bytes})
        end
      end)
    end
  end

  @doc "Messages after `seq`, oldest first."
  def since(room, seq, limit \\ 500) do
    Repo.all(from m in Message, where: m.room == ^room and m.seq > ^seq, order_by: m.seq, limit: ^limit)
  end

  @doc "Admit a device until `until`."
  def admit(key, until) do
    Repo.insert!(%Device{key: key, until: until}, on_conflict: [set: [until: until, updated_at: now()]], conflict_target: :key)
    :ok
  end

  def device_until(key) do
    case Repo.get(Device, key) do
      nil -> nil
      d -> d.until
    end
  end

  @doc "Devices whose day ended, by room — then forgotten. Old ciphertext and door records go too."
  def sweep do
    now = now()
    ended = Repo.all(from d in Device, where: d.until <= ^now, select: d.key)
    by_room = Repo.all(from rd in RoomDevice, where: rd.key in ^ended, select: {rd.room, rd.key}) |> Enum.group_by(&elem(&1, 0), &elem(&1, 1))
    Repo.delete_all(from rd in RoomDevice, where: rd.key in ^ended)
    Repo.delete_all(from d in Device, where: d.key in ^ended)
    keep = DateTime.add(now, -Application.get_env(:den, :retention_days, 30) * 86_400, :second)
    Repo.delete_all(from m in Message, where: m.inserted_at < ^keep)
    old = Date.utc_today() |> Date.add(-2) |> Date.to_iso8601()
    Repo.delete_all(from i in "issued", where: i.day < ^old)
    Repo.delete_all(from s in "spent", where: s.day < ^old)
    {by_room, ended}
  end
end
