defmodule Den.Rooms do
  @moduledoc """
  The den's state, and the only rules it applies to rooms: one commit per
  epoch (the first to arrive that checks out against the room's public MLS
  state, signed by the device that sends it), messages for the epoch a room
  is in (or a few before), and ciphertext kept for `retention_days`. It never
  holds a key.
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
      field :kind, :string, default: "open"
      field :public_state, :string
      # The member keys (hex, comma-separated) the public state names after
      # its last commit, and who made a private room.
      field :members, :string
      field :creator, :string
      # Before the last commit: undone if most members cannot apply it.
      field :prev_state, :string
      field :prev_members, :string
      field :prev_group_info, :binary
      field :last_commit_seq, :integer
      field :last_committer, :string
      field :last_commit_at, :utc_datetime
      field :info_epoch, :integer
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

  @doc "The rooms any member sees: open and proposal rooms. Private ones are known only to their members."
  def listed, do: Repo.all(from r in Room, where: r.kind != "private", order_by: [r.inserted_at, r.id], select: %{id: r.id, title: r.title, kind: r.kind})

  @doc "A room a member names, open to all members."
  def open_room(title) do
    title = title |> String.trim() |> String.slice(0, 40)
    if title == "", do: {:error, :title}, else: insert(slug(title), title, "open")
  end

  @doc """
  A private room: an unguessable id, no title, no GroupInfo ever kept. Only
  its creator may enter it until its group exists, and then only its members.
  """
  def private_room(id, creator \\ nil) do
    if id =~ ~r/\Ap-[0-9a-f]{32}\z/, do: insert(id, "", "private", creator), else: {:error, :id}
  end

  @doc "The keys the room's public state names as members (none before its group exists)."
  def members(%Room{members: m}) when is_binary(m), do: String.split(m, ",", trim: true)
  def members(%Room{public_state: state, id: id} = r) when is_binary(state) do
    keys = public_members(state, id)
    Repo.update_all(from(x in Room, where: x.id == ^id and x.epoch == ^r.epoch), set: [members: Enum.join(keys, ",")])
    keys
  end
  def members(_), do: []

  defp public_members(state, id) do
    case Den.Public.members(state, id) do
      {:ok, keys} -> keys
      _ -> []
    end
  end

  @doc """
  Whether a device may enter a room's channel and voice: any admitted device
  an open room; a private room only its members, or its creator before its
  group exists. Knowing a private room's id is not enough.
  """
  def may_enter?(%Room{kind: "private"} = r, key) do
    case members(r) do
      [] -> is_nil(r.public_state) and not is_nil(r.creator) and r.creator == key
      keys -> key in keys
    end
  end
  def may_enter?(%Room{}, _key), do: true

  @doc "Tells the room's and its voice room's channels who the members are now: anyone else leaves."
  def announce_members(room) do
    keys = members(get(room))
    DenWeb.Endpoint.broadcast("room:" <> room, "members", %{keys: keys})
    DenWeb.Endpoint.broadcast("voice:" <> room, "members", %{keys: keys})
  end

  @doc "A proposal's room: an add-on (a DAO's) opens these, listed under Proposals."
  def proposal_room(n) when is_integer(n), do: insert("proposal-#{n}", "Proposal ##{n}", "proposal")

  # Two at once for one id: the second is told it exists, not a crash.
  defp insert(id, title, kind, creator \\ nil) do
    now = now()
    row = %{id: id, title: title, kind: kind, epoch: 0, seq: 0, creator: creator, inserted_at: now, updated_at: now}

    case Repo.insert_all(Room, [row], on_conflict: :nothing, conflict_target: :id) do
      {1, _} -> {:ok, Repo.get(Room, id)}
      {0, _} -> {:error, :exists}
    end
  end

  defp slug(title) do
    base = title |> String.downcase() |> String.replace(~r/[^a-z0-9]+/, "-") |> String.trim("-") |> String.slice(0, 24)
    "r-" <> base <> "-" <> Base.encode16(:crypto.strong_rand_bytes(3), case: :lower)
  end

  @doc "Count one `what` for a device today; :ok under `limit`."
  def quota(key, what, limit, amount \\ 1) do
    day = quota_day(key)
    Repo.transaction(fn ->
      n = Repo.one(from q in "quota", where: q.key == ^key and q.day == ^day and q.what == ^what, select: q.count) || 0
      if n + amount > limit, do: Repo.rollback(:quota)
      Repo.insert_all("quota", [%{key: key, day: day, what: what, count: n + amount}], on_conflict: [set: [count: n + amount]], conflict_target: [:key, :day, :what])
    end)
    |> case do
      {:ok, _} -> :ok
      {:error, :quota} -> {:error, :quota}
    end
  end

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

  @doc """
  A room's first GroupInfo, from the member who started it: the den tracks
  the room's public state from it (every kind), and keeps the GroupInfo for
  joiners (not for a private room, which is joined by invitation).
  """
  def create(room, group_info) do
    with %Room{} = r <- get(room) || {:error, :no_room},
         true <- is_nil(r.public_state) || {:error, :exists},
         {:ok, {state, ^room, 0}} <- track(group_info) do
      keep = if r.kind == "private", do: nil, else: group_info
      keys = Enum.join(public_members(state, room), ",")
      {n, _} = Repo.update_all(from(x in Room, where: x.id == ^room and is_nil(x.public_state)), set: [public_state: state, members: keys, group_info: keep, info_epoch: 0, epoch: 0, updated_at: now()])
      if n == 1, do: :ok, else: {:error, :exists}
    else
      {:ok, _} -> {:error, :group_info}
      {:error, _} = e -> e
    end
  end

  defp track(group_info) do
    case Den.Public.track(group_info) do
      {:ok, t} -> {:ok, t}
      {:error, _} -> {:error, :group_info}
    end
  end

  @doc """
  The GroupInfo for the epoch the room is in, from any member: taken only if
  it describes the state the den tracks, so newcomers cannot be sent into
  another group. Never kept for a private room.
  """
  def info(room, epoch, group_info) do
    with %Room{kind: kind, epoch: ^epoch, public_state: state} when kind != "private" and is_binary(state) <- get(room) || {:error, :stale},
         {:ok, true} <- Den.Public.info_matches(state, group_info) do
      {n, _} = Repo.update_all(from(r in Room, where: r.id == ^room and r.epoch == ^epoch), set: [group_info: group_info, info_epoch: epoch, updated_at: now()])
      if n == 1, do: :ok, else: {:error, :stale}
    else
      {:ok, false} -> {:error, :group_info}
      {:error, :stale} -> {:error, :stale}
      {:error, _} -> {:error, :group_info}
      _ -> {:error, :stale}
    end
  end

  @doc """
  Relay `bytes` in `room`: a commit only at the room's epoch (it moves the room
  to the next), words at that epoch or a few before. Returns the message.
  """
  def relay(room, bytes, sender) do
    with {:ok, env} <- Den.Mls.envelope(bytes),
         true <- env.room == room || {:error, :wrong_room} do
      Repo.transaction(fn ->
        r = Repo.get!(Room, room)

        cond do
          r.kind == "private" and env.external ->
            Repo.rollback(:private)

          is_nil(r.public_state) ->
            Repo.rollback({:no_group, r.epoch})

          env.commit and env.epoch != r.epoch ->
            Repo.rollback({:epoch, r.epoch})

          not env.commit and (env.epoch > r.epoch or env.epoch < r.epoch - @past_epochs) ->
            Repo.rollback({:epoch, r.epoch})

          env.commit and barred?(r.id, sender) ->
            Repo.rollback(:barred)

          env.commit ->
            # Checked against the room's public state, and signed by the
            # device whose session sent it: no commit-shaped bytes, and no
            # commits in another device's name.
            case Den.Public.commit(r.public_state, bytes) do
              {:ok, {state, epoch, ^sender}} when epoch == r.epoch + 1 -> order(r, env, bytes, epoch, state, sender)
              {:ok, {_, _, _}} -> Repo.rollback(:not_yours)
              {:error, why} -> Repo.rollback({:bad_commit, why})
            end

          # Words only from a member: a device the last commit took out (or
          # never let in) cannot write at an earlier epoch in a newcomer's seat.
          sender not in members(r) ->
            Repo.rollback(:not_member)

          true ->
            order(r, env, bytes, r.epoch, r.public_state)
        end
      end)
    end
  end

  defp order(r, env, bytes, epoch, state, committer \\ nil) do
    seq = r.seq + 1
    keys = if env.commit, do: Enum.join(public_members(state, r.id), ","), else: r.members
    # A commit keeps what it replaces, until the next one.
    before =
      if env.commit,
        do: [prev_state: r.public_state, prev_members: r.members || Enum.join(members(r), ","), prev_group_info: r.group_info, last_commit_seq: seq, last_committer: committer, last_commit_at: now()],
        else: []
    # Optimistic: the room must still be where we read it.
    {1, _} = Repo.update_all(from(x in Room, where: x.id == ^r.id and x.seq == ^r.seq), set: [seq: seq, epoch: epoch, public_state: state, members: keys, updated_at: now()] ++ before)
    Repo.insert!(%Message{room: r.id, seq: seq, epoch: env.epoch, commit: env.commit, bytes: bytes})
  end

  @doc """
  A member reports that it cannot apply the room's last commit (`seq`): its
  signature over `"den-report-v1|" <> room <> "|" <> seq` by its own key. The
  den cannot check a commit's secrets, so it takes the members' word, from
  members in the room both before and after that commit other than its
  sender. When most of those members report it, the commit is undone: the
  room goes back to the state before it (and the messages after it go), and
  its sender may not commit in the room for a day.

  Returns `{:ok, :rolled_back, epoch, seq}` or `{:ok, :noted}`.
  """
  def report(room, seq, key, sig_hex) do
    with %Room{} = r <- get(room) || {:error, :no_room},
         true <- (is_integer(seq) and seq == r.last_commit_seq) || {:error, :stale},
         true <- not is_nil(r.prev_state) || {:error, :stale},
         before = String.split(r.prev_members || "", ",", trim: true),
         now_in = members(r),
         true <- (key in before and key in now_in and key != r.last_committer) || {:error, :not_member},
         true <- signed?(key, "den-report-v1|#{room}|#{seq}", sig_hex) || {:error, :signature} do
      Repo.insert_all("reports", [%{room: room, seq: seq, key: key, inserted_at: now()}], on_conflict: :nothing)
      voters = Enum.filter(before, &(&1 in now_in and &1 != r.last_committer))
      n = Repo.aggregate(from(x in "reports", where: x.room == ^room and x.seq == ^seq), :count)
      if n * 2 > length(voters), do: roll_back(r, true), else: {:ok, :noted}
    end
  end

  @doc """
  An open room's last commit whose GroupInfo never came: honest devices send
  it straight after committing, and without it nobody can join the room again
  at its new epoch. After `seconds` the commit is undone (its sender is not
  barred: the GroupInfo may have been lost on the way). Returns the rooms
  undone, as {room, epoch, seq}.
  """
  def undo_unannounced(seconds \\ 30) do
    cutoff = DateTime.add(now(), -seconds, :second)

    Repo.all(from r in Room, where: r.kind != "private" and not is_nil(r.last_commit_seq) and not is_nil(r.prev_state) and r.last_commit_at < ^cutoff and (is_nil(r.info_epoch) or r.info_epoch < r.epoch))
    |> Enum.flat_map(fn r ->
      case roll_back(r, false) do
        {:ok, :rolled_back, epoch, seq} -> [{r.id, epoch, seq}]
        _ -> []
      end
    end)
  end

  defp roll_back(r, bar?) do
    Repo.transaction(fn ->
      seq = r.last_commit_seq - 1
      {n, _} =
        Repo.update_all(from(x in Room, where: x.id == ^r.id and x.last_commit_seq == ^r.last_commit_seq and x.seq >= ^r.last_commit_seq),
          set: [public_state: r.prev_state, members: r.prev_members, group_info: r.prev_group_info, info_epoch: if(r.prev_group_info, do: r.epoch - 1), epoch: r.epoch - 1, seq: seq,
                prev_state: nil, prev_members: nil, prev_group_info: nil, last_commit_seq: nil, last_commit_at: nil, updated_at: now()])
      if n != 1, do: Repo.rollback(:stale)
      Repo.delete_all(from m in Message, where: m.room == ^r.id and m.seq > ^seq)
      Repo.delete_all(from x in "reports", where: x.room == ^r.id)
      if bar? and r.last_committer do
        until = DateTime.add(now(), 86_400, :second)
        Repo.insert_all("barred", [%{room: r.id, key: r.last_committer, until: until}], on_conflict: [set: [until: until]], conflict_target: [:room, :key])
      end
      {:rolled_back, r.epoch - 1, seq}
    end)
    |> case do
      {:ok, {:rolled_back, epoch, seq}} -> {:ok, :rolled_back, epoch, seq}
      {:error, why} -> {:error, why}
    end
  end

  defp barred?(room, key), do: Repo.exists?(from b in "barred", where: b.room == ^room and b.key == ^key and b.until > ^now())

  defp signed?(key, msg, sig_hex) do
    with {:ok, sig} when byte_size(sig) == 64 <- Base.decode16(sig_hex || "", case: :lower),
         {:ok, pk} <- Base.decode16(key, case: :lower) do
      :crypto.verify(:eddsa, :none, msg, sig, [pk, :ed25519])
    else
      _ -> false
    end
  end

  @doc "Messages after `seq`, oldest first."
  def since(room, seq, limit \\ 500) do
    Repo.all(from m in Message, where: m.room == ^room and m.seq > ^seq, order_by: m.seq, limit: ^limit)
  end

  # A device's quotas run for the day its token was for, the grace hour
  # after midnight included: not a fresh day's worth in that hour.
  defp quota_day(key) do
    case device_until(key) do
      %DateTime{} = until -> until |> DateTime.add(-3600, :second) |> DateTime.to_date() |> Date.add(-1) |> Date.to_iso8601()
      _ -> Date.to_iso8601(Date.utc_today())
    end
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

  @doc """
  Devices whose day ended, by room — then forgotten. Old ciphertext, files
  and door records go too. One transaction, so a device that enters again
  while the sweep runs is either swept (before) or kept (after), never half.
  """
  def sweep do
    {:ok, result} =
      Repo.transaction(fn ->
        now = now()
        ended = Repo.all(from d in Device, where: d.until <= ^now, select: d.key)
        by_room =
          ended
          |> Enum.chunk_every(500)
          |> Enum.flat_map(&Repo.all(from rd in RoomDevice, where: rd.key in ^&1, select: {rd.room, rd.key}))
          |> Enum.group_by(&elem(&1, 0), &elem(&1, 1))

        for keys <- Enum.chunk_every(ended, 500) do
          Repo.delete_all(from rd in RoomDevice, where: rd.key in ^keys)
          Repo.delete_all(from d in Device, where: d.key in ^keys)
          Repo.delete_all(from k in "key_packages", where: k.key in ^keys)
        end

        keep = DateTime.add(now, -Application.get_env(:den, :retention_days, 30) * 86_400, :second)
        Repo.delete_all(from m in Message, where: m.inserted_at < ^keep)
        old = Date.utc_today() |> Date.add(-2) |> Date.to_iso8601()
        Repo.delete_all(from i in "issued", where: i.day < ^old)
        Repo.delete_all(from s in "spent", where: s.day < ^old)
        Repo.delete_all(from q in "quota", where: q.day < ^old)
        Repo.delete_all(from b in "barred", where: b.until < ^now)
        Repo.delete_all(from i in "inbox", where: i.inserted_at < ^keep)
        old_blobs = Repo.all(from b in "blobs", where: b.inserted_at < ^keep, select: b.id)
        for ids <- Enum.chunk_every(old_blobs, 500), do: Repo.delete_all(from b in "blobs", where: b.id in ^ids)
        {by_room, ended, old_blobs}
      end)

    {by_room, ended, old_blobs} = result
    # The files themselves once their rows are gone.
    for id <- old_blobs, do: File.rm(Path.join(Den.Blobs.dir(), id))
    {by_room, ended}
  end
end
