defmodule Den.Mod do
  @moduledoc """
  Who may do what in the den, Discord's way: one owner device with every
  power (named by the host: `epeaky owner m-…`), roles with permissions the
  owner (or a role allowed to) makes and gives, and what moderators do to
  devices: timeouts, kicks and bans. All by device key, the one identity the
  den authenticates; it never learns which devices are one person, nor which
  pass a device came in with.

  Roles have a position. A device acts only on devices whose highest role is
  below its own, and makes, edits, gives and takes only roles below its own;
  it never grants a permission it does not hold. The owner is above all.

  Also kept here: the den's own settings (owner, name, description, icon,
  limits) and the audit log of what moderators did (never what anyone wrote).

  With no owner, nobody holds any permission and the den works as it did
  before roles: see `governed?/0`.
  """
  import Ecto.Query
  alias Den.Repo

  @perms ~w(administrator manage_server manage_channels manage_roles manage_messages timeout kick ban manage_invites bypass_slow mention_everyone)
  @everyone "everyone"
  @owner_top 1_000_000
  @max_roles 50
  @slow [0, 5, 10, 15, 30, 60, 120, 300, 600, 3600, 21_600]
  @timeout_max 28 * 86_400
  @audit_days 90
  @icon_max 262_144
  @icon_types ~w(image/png image/jpeg image/webp)
  # Of DEN_LIMITS, what Manage server may lower (never above the host's).
  @limit_keys ~w(file_max file_bytes message_bytes files)
  @mod_perms ~w(administrator manage_server manage_roles manage_channels manage_messages timeout kick ban manage_invites)

  def perm_names, do: @perms
  def slow_choices, do: @slow
  def everyone, do: @everyone

  # ---- What the den holds, read once and kept until it changes ----------------

  defp state do
    if Application.get_env(:den, :mod_cache, true) do
      case :persistent_term.get({__MODULE__, :state}, nil) do
        nil -> tap(load(), &:persistent_term.put({__MODULE__, :state}, &1))
        s -> s
      end
    else
      load()
    end
  end

  defp changed do
    :persistent_term.erase({__MODULE__, :state})
    Den.Limits.forget()
    :ok
  end

  defp load do
    roles =
      Repo.all(from r in "roles", select: %{id: r.id, name: r.name, colour: r.colour, position: r.position, hoist: type(r.hoist, :boolean), perms: r.perms})
      |> Enum.map(&%{&1 | perms: split(&1.perms)})

    everyone = Enum.find(roles, &(&1.id == @everyone)) || %{id: @everyone, name: "@everyone", colour: nil, position: 0, hoist: false, perms: []}
    roles = [%{everyone | position: 0, name: "@everyone"} | Enum.reject(roles, &(&1.id == @everyone))]
    holders = Repo.all(from rk in "role_keys", select: {rk.key, rk.role}) |> Enum.group_by(&elem(&1, 0), &elem(&1, 1))
    sanctions =
      Repo.all(from s in "sanctions", select: %{key: s.key, kind: s.kind, until: type(s.until, :utc_datetime), by: s.by, reason: s.reason, at: type(s.at, :utc_datetime)})
      |> Map.new(&{&1.key, &1})
    settings = Repo.all(from s in "settings", select: {s.key, s.value}) |> Map.new()
    %{roles: Map.new(roles, &{&1.id, &1}), holders: holders, sanctions: sanctions, settings: settings}
  end

  defp split(nil), do: []
  defp split(s), do: s |> String.split(",", trim: true) |> Enum.filter(&(&1 in @perms))

  defp now, do: DateTime.utc_now() |> DateTime.truncate(:second)
  defp key?(k), do: is_binary(k) and k =~ ~r/\A[0-9a-f]{64}\z/

  # ---- The owner -----------------------------------------------------------------

  def owner, do: state().settings["owner"]
  def owner?(key), do: is_binary(key) and key == owner()

  @doc "Whether the den has an owner, and so runs by roles."
  def governed?, do: not is_nil(owner())

  @doc "The host names the owner device (or none, with nil)."
  def set_owner(nil) do
    Repo.delete_all(from s in "settings", where: s.key == "owner")
    log(nil, "owner cleared", nil, nil)
    changed()
  end

  def set_owner(key) do
    true = key?(key)
    put_setting("owner", key)
    log(nil, "owner set", key, nil)
    changed()
  end

  @doc "The owner hands ownership to another device."
  def owner_pass(actor, key) do
    with true <- owner?(actor) || {:error, :forbidden},
         true <- (key?(key) and key != actor) || {:error, :key} do
      put_setting("owner", key)
      log(actor, "owner passed", key, nil)
      changed()
    end
  end

  # ---- Permissions and order -------------------------------------------------

  def roles, do: state().roles |> Map.values() |> Enum.sort_by(&{-&1.position, &1.name})
  def role(id), do: Map.get(state().roles, id)
  def roles_of(key), do: Map.get(state().holders, key, [])

  @doc "A device's permissions: every one for the owner and for Administrator."
  def perms(key) do
    s = state()

    if owner?(key) do
      MapSet.new(@perms)
    else
      ps = [@everyone | Map.get(s.holders, key, [])] |> Enum.flat_map(&Map.get(s.roles, &1, %{perms: []}).perms) |> MapSet.new()
      if "administrator" in ps, do: MapSet.new(@perms), else: ps
    end
  end

  def can?(key, perm), do: perm in perms(key)

  @doc "A device's highest role's position (the owner's above every role)."
  def top(key) do
    if owner?(key), do: @owner_top, else: roles_of(key) |> Enum.map(&(role(&1) || %{position: 0}).position) |> Enum.max(fn -> 0 end)
  end

  @doc "Whether `actor` may act on `target`: its highest role is above the target's."
  def above?(actor, target), do: not owner?(target) and (owner?(actor) or top(actor) > top(target))

  defp may(actor, perm), do: if(can?(actor, perm), do: :ok, else: {:error, :forbidden})

  defp on(actor, key) do
    cond do
      not key?(key) -> {:error, :key}
      key == actor -> {:error, :self}
      not above?(actor, key) -> {:error, :above}
      true -> :ok
    end
  end

  # A role `actor` may make, edit, give or take: below its own highest.
  defp below?(actor, position), do: owner?(actor) or position < top(actor)

  # ---- Roles ---------------------------------------------------------------------

  @doc """
  Make or change a role: `%{"id" (none for a new one), "name", "colour"
  ("#rrggbb" or nil), "hoist" (shown apart in the member list), "perms",
  "position"}`. @everyone takes only permissions.
  """
  def role_save(actor, attrs) when is_map(attrs) do
    id = attrs["id"]
    old = id && role(id)

    with :ok <- may(actor, "manage_roles"),
         true <- (is_nil(id) or not is_nil(old)) || {:error, :role},
         true <- (is_nil(old) or old.id == @everyone or below?(actor, old.position)) || {:error, :above},
         {:ok, perms} <- perms_in(attrs["perms"]),
         added = perms -- ((old && old.perms) || []),
         true <- (owner?(actor) or Enum.all?(added, &can?(actor, &1))) || {:error, :perm} do
      if old && old.id == @everyone do
        save_role(%{old | perms: perms})
      else
        with {:ok, name} <- text(attrs["name"], 1, 32, :name),
             {:ok, colour} <- colour(attrs["colour"]),
             position = attrs["position"] || (old && old.position) || 1,
             true <- (is_integer(position) and position >= 1 and position < @owner_top) || {:error, :position},
             true <- below?(actor, position) || {:error, :above},
             true <- (old != nil or map_size(state().roles) <= @max_roles) || {:error, :limit} do
          r = %{id: id || "r-" <> Base.encode16(:crypto.strong_rand_bytes(6), case: :lower), name: name, colour: colour, position: position, hoist: attrs["hoist"] == true, perms: perms}
          log(actor, if(old, do: "role edited", else: "role made"), r.id, name)
          save_role(r)
        end
      end
    end
  end

  def role_save(_, _), do: {:error, :role}

  defp save_role(r) do
    row = %{id: r.id, name: r.name, colour: r.colour, position: r.position, hoist: r.hoist, perms: Enum.join(r.perms, ","), inserted_at: now()}
    Repo.insert_all("roles", [row], on_conflict: {:replace, [:name, :colour, :position, :hoist, :perms]}, conflict_target: :id)
    changed()
    {:ok, role(r.id)}
  end

  defp perms_in(nil), do: {:ok, []}
  defp perms_in(list) when is_list(list), do: if(Enum.all?(list, &(&1 in @perms)), do: {:ok, Enum.uniq(list)}, else: {:error, :perm})
  defp perms_in(_), do: {:error, :perm}

  defp colour(nil), do: {:ok, nil}
  defp colour(""), do: {:ok, nil}
  defp colour(c) when is_binary(c), do: if(c =~ ~r/\A#[0-9a-f]{6}\z/i, do: {:ok, String.downcase(c)}, else: {:error, :colour})
  defp colour(_), do: {:error, :colour}

  defp text(s, min, max, why) when is_binary(s) do
    t = s |> String.replace(~r/[\x00-\x1f\x7f\x{200B}-\x{200F}\x{202A}-\x{202E}\x{2060}-\x{2069}\x{FEFF}\x{061C}]/u, " ") |> String.replace(~r/\s+/u, " ") |> String.trim()
    if String.length(t) >= min and String.length(t) <= max, do: {:ok, t}, else: {:error, why}
  end

  defp text(nil, 0, _, _), do: {:ok, ""}
  defp text(_, _, _, why), do: {:error, why}

  def role_delete(actor, id) do
    with :ok <- may(actor, "manage_roles"),
         %{} = r <- (id != @everyone && role(id)) || {:error, :role},
         true <- below?(actor, r.position) || {:error, :above} do
      Repo.delete_all(from x in "roles", where: x.id == ^id)
      Repo.delete_all(from x in "role_keys", where: x.role == ^id)
      log(actor, "role deleted", id, r.name)
      changed()
    end
  end

  def role_give(actor, key, id), do: holder(actor, key, id, :give)
  def role_take(actor, key, id), do: holder(actor, key, id, :take)

  defp holder(actor, key, id, how) do
    with :ok <- may(actor, "manage_roles"),
         %{} = r <- (id != @everyone && role(id)) || {:error, :role},
         true <- below?(actor, r.position) || {:error, :above},
         true <- key?(key) || {:error, :key},
         true <- (key == actor or above?(actor, key)) || {:error, :above} do
      case how do
        :give -> Repo.insert_all("role_keys", [%{role: id, key: key}], on_conflict: :nothing)
        :take -> Repo.delete_all(from x in "role_keys", where: x.role == ^id and x.key == ^key)
      end

      log(actor, if(how == :give, do: "role given", else: "role taken"), key, r.name)
      changed()
    end
  end

  # ---- Timeouts, kicks, bans -------------------------------------------------

  @doc "What holds a device now: %{kind: \"timeout\" | \"kick\" | \"ban\", until, …}, or nil."
  def sanction(key) do
    case Map.get(state().sanctions, key) do
      %{until: nil} = s -> s
      %{until: u} = s -> if DateTime.compare(u, DateTime.utc_now()) == :gt, do: s, else: nil
      nil -> nil
    end
  end

  def banned?(key), do: match?(%{kind: "ban"}, sanction(key))
  @doc "Kicked or banned: out of the open rooms, refused at the door and on every channel."
  def out?(key), do: match?(%{kind: k} when k in ["kick", "ban"], sanction(key))
  @doc "May not write, react, type or speak (timed out, kicked or banned)."
  def muted?(key), do: not is_nil(sanction(key))

  def timeout(actor, key, seconds, reason \\ nil) do
    with :ok <- may(actor, "timeout"),
         :ok <- on(actor, key),
         true <- (is_integer(seconds) and seconds >= 60 and seconds <= @timeout_max) || {:error, :duration},
         {:ok, reason} <- text(reason || "", 0, 300, :reason) do
      sanction!(key, "timeout", DateTime.add(now(), seconds, :second), actor, reason)
      log(actor, "timeout #{seconds}s", key, reason)
      changed()
    end
  end

  def untimeout(actor, key) do
    with :ok <- may(actor, "timeout"), :ok <- on(actor, key) do
      Repo.delete_all(from s in "sanctions", where: s.key == ^key and s.kind == "timeout")
      log(actor, "timeout removed", key, nil)
      changed()
    end
  end

  @doc "Out of every open room until the next UTC midnight (at least an hour): it may come back after."
  def kick(actor, key, reason \\ nil) do
    with :ok <- may(actor, "kick"),
         :ok <- on(actor, key),
         {:ok, reason} <- text(reason || "", 0, 300, :reason) do
      midnight = Date.utc_today() |> Date.add(1) |> DateTime.new!(~T[00:00:00])
      until = Enum.max([midnight, DateTime.add(now(), 3600, :second)], DateTime)
      sanction!(key, "kick", until, actor, reason)
      log(actor, "kick", key, reason)
      changed()
    end
  end

  def ban(actor, key, reason \\ nil) do
    with :ok <- may(actor, "ban"),
         :ok <- on(actor, key),
         {:ok, reason} <- text(reason || "", 0, 300, :reason) do
      sanction!(key, "ban", nil, actor, reason)
      log(actor, "ban", key, reason)
      changed()
    end
  end

  def unban(actor, key) do
    with :ok <- may(actor, "ban"), true <- key?(key) || {:error, :key} do
      Repo.delete_all(from s in "sanctions", where: s.key == ^key and s.kind in ["ban", "kick"])
      log(actor, "unban", key, nil)
      changed()
    end
  end

  # A stronger sanction is never replaced by a weaker one while it holds.
  @strength %{"timeout" => 1, "kick" => 2, "ban" => 3}
  defp sanction!(key, kind, until, by, reason) do
    case sanction(key) do
      %{kind: held} when :erlang.map_get(held, @strength) > :erlang.map_get(kind, @strength) -> :ok
      _ ->
        row = %{key: key, kind: kind, until: until, by: by, reason: reason, at: now()}
        Repo.insert_all("sanctions", [row], on_conflict: {:replace, [:kind, :until, :by, :reason, :at]}, conflict_target: :key)
    end
  end

  @doc "Bans in force, for those who may ban."
  def bans do
    for {_, s} <- state().sanctions, s.kind == "ban", do: Map.take(s, [:key, :by, :reason, :at])
  end

  # ---- The den's settings --------------------------------------------------------

  def settings, do: state().settings

  defp put_setting(k, v) do
    Repo.insert_all("settings", [%{key: k, value: v}], on_conflict: {:replace, [:value]}, conflict_target: :key)
  end

  @doc """
  Change the den's name, description, icon (a data: URL of a PNG, JPEG or
  WebP image up to 256 KB, or "" for none) and limits (each at most what the
  host set).
  """
  def settings_save(actor, attrs) when is_map(attrs) do
    with :ok <- may(actor, "manage_server"),
         {:ok, changes} <- settings_in(attrs) do
      for {k, v} <- changes do
        if v in [nil, ""], do: Repo.delete_all(from s in "settings", where: s.key == ^k), else: put_setting(k, v)
      end

      log(actor, "settings changed", nil, changes |> Map.keys() |> Enum.join(", "))
      changed()
    end
  end

  def settings_save(_, _), do: {:error, :settings}

  defp settings_in(attrs) do
    Enum.reduce_while(attrs, {:ok, %{}}, fn
      {"name", v}, {:ok, acc} -> step(text(v, 0, 40, :name), &Map.put(acc, "name", &1))
      {"description", v}, {:ok, acc} -> step(text(v, 0, 300, :description), &Map.put(acc, "description", &1))
      {"icon", v}, {:ok, acc} -> step(icon(v), &Map.put(acc, "icon", &1))
      {"limits", v}, {:ok, acc} -> step(limits_in(v), &Map.put(acc, "limits", &1))
      _, _ -> {:halt, {:error, :settings}}
    end)
  end

  defp step({:ok, v}, f), do: {:cont, {:ok, f.(v)}}
  defp step({:error, _} = e, _), do: {:halt, e}

  defp icon(""), do: {:ok, ""}

  defp icon("data:" <> rest) do
    with [type, b64] <- String.split(rest, ";base64,", parts: 2),
         true <- type in @icon_types,
         {:ok, bytes} when byte_size(bytes) <= @icon_max <- Base.decode64(b64),
         true <- magic?(type, bytes) do
      {:ok, "data:" <> rest}
    else
      _ -> {:error, :icon}
    end
  end

  defp icon(_), do: {:error, :icon}

  defp magic?("image/png", <<0x89, "PNG", _::binary>>), do: true
  defp magic?("image/jpeg", <<0xFF, 0xD8, _::binary>>), do: true
  defp magic?("image/webp", <<"RIFF", _::binary-size(4), "WEBP", _::binary>>), do: true
  defp magic?(_, _), do: false

  defp limits_in(m) when is_map(m) do
    if Enum.all?(m, fn {k, v} -> k in @limit_keys and is_integer(v) and v > 0 end),
      do: {:ok, Jason.encode!(m)},
      else: {:error, :limits}
  end

  defp limits_in(_), do: {:error, :limits}

  @doc "Limits Manage server set, as atoms (Den.Limits caps each by the host's)."
  def limits do
    case state().settings["limits"] do
      nil -> %{}
      j -> for {k, v} <- Jason.decode!(j), k in @limit_keys, into: %{}, do: {String.to_existing_atom(k), v}
    end
  end

  @doc "The den's icon: {type, bytes} or nil."
  def icon_bytes do
    with "data:" <> rest <- state().settings["icon"],
         [type, b64] <- String.split(rest, ";base64,", parts: 2),
         {:ok, bytes} <- Base.decode64(b64) do
      {type, bytes}
    else
      _ -> nil
    end
  end

  # ---- Open rooms' settings --------------------------------------------------

  @doc """
  An open room's settings: `%{"title", "topic", "slow" (seconds, one of
  #{inspect(@slow)}), "post_roles" (role ids; [] for everyone), "position"}`,
  each optional.
  """
  def room_save(actor, id, attrs) when is_map(attrs) do
    with :ok <- may(actor, "manage_channels"),
         %{kind: "open"} <- Den.Rooms.get(id) || {:error, :room},
         {:ok, set} <- room_in(attrs) do
      Repo.update_all(from(r in "rooms", where: r.id == ^id), set: Keyword.new(set))
      if Map.has_key?(set, :title), do: put_setting("named:" <> id, "1")
      log(actor, "channel changed", id, set |> Map.keys() |> Enum.join(", "))
      changed()
      {:ok, Den.Rooms.get(id)}
    else
      %{} -> {:error, :room}
      e -> e
    end
  end

  def room_save(_, _, _), do: {:error, :room}

  defp room_in(attrs) do
    Enum.reduce_while(attrs, {:ok, %{}}, fn
      {"title", v}, {:ok, acc} -> step(text(v, 1, 40, :title), &Map.put(acc, :title, &1))
      {"topic", v}, {:ok, acc} -> step(text(v || "", 0, 200, :topic), &Map.put(acc, :topic, &1))
      {"slow", v}, {:ok, acc} -> step(if(v in @slow, do: {:ok, v}, else: {:error, :slow}), &Map.put(acc, :slow_seconds, &1))
      {"post_roles", v}, {:ok, acc} -> step(post_roles(v), &Map.put(acc, :post_roles, &1))
      {"position", v}, {:ok, acc} -> step(if(is_integer(v) and v >= 0 and v < 10_000, do: {:ok, v}, else: {:error, :position}), &Map.put(acc, :position, &1))
      _, _ -> {:halt, {:error, :room}}
    end)
  end

  defp post_roles([]), do: {:ok, nil}
  defp post_roles(ids) when is_list(ids), do: if(Enum.all?(ids, &(is_binary(&1) and role(&1) != nil and &1 != @everyone)), do: {:ok, Enum.join(Enum.uniq(ids), ",")}, else: {:error, :role})
  defp post_roles(_), do: {:error, :role}

  @doc "Whether a host-named room was renamed by a moderator (so starting again keeps that name)."
  def renamed?(id), do: Map.has_key?(state().settings, "named:" <> id)

  @doc "Whether `key` may write in `room` (a room's \"Who may post\")."
  def may_post?(%{post_roles: nil}, _), do: true
  def may_post?(%{post_roles: ""}, _), do: true
  def may_post?(%{post_roles: ids}, key) do
    owner?(key) or can?(key, "administrator") or Enum.any?(String.split(ids, ","), &(&1 in roles_of(key)))
  end

  def may_post?(_, _), do: true

  # ---- The audit log ---------------------------------------------------------------

  def log(actor, action, target, reason) do
    Repo.insert_all("audit", [%{at: now(), actor: actor, action: action, target: target, reason: if(reason in [nil, ""], do: nil, else: reason)}])
    :ok
  end

  @doc "May read the audit log: any moderator's permission."
  def moderator?(key), do: Enum.any?(@mod_perms, &can?(key, &1))

  def audit(n \\ 100) do
    Repo.all(from a in "audit", order_by: [desc: a.id], limit: ^n, select: %{at: type(a.at, :utc_datetime), actor: a.actor, action: a.action, target: a.target, reason: a.reason})
  end

  def sweep do
    keep = DateTime.add(now(), -@audit_days * 86_400, :second)
    Repo.delete_all(from a in "audit", where: a.at < ^keep)
    {n, _} = Repo.delete_all(from s in "sanctions", where: not is_nil(s.until) and s.until < ^now())
    if n > 0, do: changed()
    :ok
  end

  # ---- What every member's page is told ---------------------------------------------

  @doc """
  The den's roles, who holds them, the sanctions in force (no reasons), and its
  name and description: what members see, as on Discord.
  """
  def public do
    s = state()

    %{
      owner: owner(),
      roles: for(r <- roles(), do: Map.take(r, [:id, :name, :colour, :position, :hoist, :perms])),
      holders: s.holders,
      sanctions: for({k, _} <- s.sanctions, x = sanction(k), x != nil, do: %{key: k, kind: x.kind, until: x.until && DateTime.to_unix(x.until)}),
      den: %{name: s.settings["name"] || "", description: s.settings["description"] || "", icon: Map.has_key?(s.settings, "icon")},
      limits: limits()
    }
  end

  @doc "What one device is told on joining: the public state and its own permissions."
  def for_device(key), do: Map.put(public(), :mine, perms(key) |> MapSet.to_list() |> Enum.sort())
end
