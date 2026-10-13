defmodule Den.Gate.Members do
  @moduledoc """
  The den's own member list, the default gate. It keeps pass hashes, never
  codes: a member's device makes its code and sends only the hash
  (`hash/1`) when it joins, so the den learns the code first when the door
  signs a token with it. Members join in the ways the host opens
  (`config :den, entry: [...]`, or `DEN_ENTRY`):

    * `:invite` (the default): a one-use link the host made (`Den.Host`).
    * `:approval`: someone asks with a short note and the host approves.
    * `:external`: the host's own site registers passes with a bearer token
      only the host has (`DEN_ISSUER_TOKEN`), after checking whatever it
      requires.
  """
  @behaviour Den.Gate
  import Ecto.Query
  alias Den.Repo

  @domain "den.pass.v1"
  @note_max 280
  @pending_max 200
  @pending_per_source 3

  @doc "The hash a code makes: what the member list keeps."
  def hash(<<code::binary-size(32)>>), do: :crypto.hash(:sha256, <<0>> <> @domain <> code)

  @impl true
  def id(code), do: hash(code)

  @impl true
  def live?(code), do: {:ok, Repo.exists?(from m in "members", where: m.pass == ^hex(hash(code)))}

  @doc "The ways in that are open."
  def entry, do: Application.get_env(:den, :entry, [:invite])
  def open?(way), do: way in entry()

  # ---- Invite links -----------------------------------------------------------

  @doc "Makes `n` one-use invite codes, good for `days` days. Returns the codes; only their hashes are kept."
  def invite(n, days) when n in 1..100 and days in 1..90 do
    expires = DateTime.add(now(), days * 86_400, :second)

    for _ <- 1..n do
      code = :crypto.strong_rand_bytes(32)
      Repo.insert_all("invites", [%{id: hex(:crypto.hash(:sha256, code)), made_at: now(), expires_at: expires}])
      hex(code)
    end
  end

  @doc "Joins with an invite code and the hash of the member's own pass code. The invite is used up."
  def join(invite_hex, pass_hex) do
    with true <- open?(:invite) || {:error, :closed},
         {:ok, invite} <- Den.Gate.code(invite_hex) |> ok_or(:invite),
         {:ok, pass} <- pass_hash(pass_hex) do
      id = hex(:crypto.hash(:sha256, invite))

      Repo.transaction(fn ->
        n = Repo.update_all(from(i in "invites", where: i.id == ^id and is_nil(i.used_at) and is_nil(i.revoked_at) and i.expires_at > ^now()), set: [used_at: now()])

        case n do
          {1, _} -> add!(pass, "invite", id)
          _ -> Repo.rollback(:invite)
        end
      end)
      |> case do
        {:ok, :ok} -> :ok
        {:error, why} -> {:error, why}
      end
    end
  end

  # ---- The host's approval ----------------------------------------------------

  @doc """
  Asks to join: the hash of the asker's pass code, a note for the host, and a
  keyed tag of the address it came from (`DenWeb.RateLimit.tag/1`), so one
  address cannot fill the queue: at most #{3} waiting from one source, #{200}
  in all. The tag is forgotten with the note. Returns the request's id.
  """
  def ask(pass_hex, note, source \\ nil) do
    note = String.trim(note || "")

    with true <- open?(:approval) || {:error, :closed},
         {:ok, pass} <- pass_hash(pass_hex),
         true <- (note != "" and String.length(note) <= @note_max) || {:error, :note},
         true <- is_nil(source) or Repo.aggregate(from(r in "requests", where: is_nil(r.decided) and r.source == ^source), :count) < @pending_per_source || {:error, :busy},
         true <- Repo.aggregate(from(r in "requests", where: is_nil(r.decided)), :count) < @pending_max || {:error, :busy} do
      id = hex(:crypto.strong_rand_bytes(16))
      Repo.insert_all("requests", [%{id: id, pass: pass, note: note, source: source, asked_at: now()}])
      {:ok, id}
    end
  end

  @doc "The host turns down every request still waiting. Returns how many."
  def decline_all do
    {n, _} = Repo.update_all(from(r in "requests", where: is_nil(r.decided)), set: [decided: "declined", note: nil, source: nil])
    n
  end

  @doc "Where a request stands: \"pending\", \"approved\" or \"declined\"."
  def status(id) do
    case Repo.one(from r in "requests", where: r.id == ^to_string(id), select: r.decided) do
      nil -> if Repo.exists?(from r in "requests", where: r.id == ^to_string(id)), do: {:ok, "pending"}, else: {:error, :request}
      decided -> {:ok, decided}
    end
  end

  @doc "Requests waiting for the host, oldest first."
  def pending, do: Repo.all(from r in "requests", where: is_nil(r.decided), order_by: r.asked_at, select: %{id: r.id, note: r.note, asked_at: r.asked_at})

  @doc "The host lets a request in. Its note is forgotten."
  def approve(id), do: decide(id, "approved")
  @doc "The host turns a request down. Its note is forgotten."
  def decline(id), do: decide(id, "declined")

  defp decide(id, how) do
    Repo.transaction(fn ->
      case Repo.one(from r in "requests", where: r.id == ^to_string(id) and is_nil(r.decided), select: r.pass) do
        nil -> Repo.rollback(:request)
        pass ->
          Repo.update_all(from(r in "requests", where: r.id == ^to_string(id)), set: [decided: how, note: nil, source: nil])
          if how == "approved", do: add!(pass, "approval", to_string(id)), else: :ok
      end
    end)
    |> case do
      {:ok, :ok} -> :ok
      {:error, why} -> {:error, why}
    end
  end

  # ---- Passes the host's site registers ---------------------------------------

  @doc "Registers the hash of a pass code, for the host's own site (bearer `DEN_ISSUER_TOKEN`)."
  def register(pass_hex) do
    with true <- open?(:external) || {:error, :closed}, {:ok, pass} <- pass_hash(pass_hex) do
      add!(pass, "external", nil)
    end
  end

  @doc "Removes a pass: the member can no longer get day tokens (entry ends with the current day)."
  def remove(pass_hex) do
    with {:ok, pass} <- pass_hash(pass_hex) do
      case Repo.delete_all(from m in "members", where: m.pass == ^pass) do
        {1, _} -> :ok
        _ -> {:error, :pass}
      end
    end
  end

  @doc "Whether a bearer token is the host's issuer token (compared in constant time)."
  def issuer?(token) do
    case Application.get_env(:den, :issuer_token) do
      t when is_binary(t) and byte_size(t) >= 32 and is_binary(token) -> Plug.Crypto.secure_compare(t, token)
      _ -> false
    end
  end

  @doc """
  A new den's first way in: with nobody on the member list and no invite made
  yet, one invite for 30 days, so the host can enter. Returns its code, or nil.
  """
  def first_invite do
    if Den.Gate.module() == __MODULE__ and open?(:invite) and not Repo.exists?(from(m in "members")) and not Repo.exists?(from(i in "invites")) do
      hd(invite(1, 30))
    end
  end

  # ---- The host's view ----------------------------------------------------------

  @doc "Invites: id prefix, when made, until when, whether used or withdrawn."
  def invites, do: Repo.all(from i in "invites", order_by: i.made_at, select: %{id: i.id, made_at: i.made_at, expires_at: i.expires_at, used_at: i.used_at, revoked_at: i.revoked_at})

  @doc "Withdraws an invite (by its id or a unique prefix of it), and the pass it let in, if any."
  def withdraw(prefix) when is_binary(prefix) and byte_size(prefix) >= 8 do
    case Repo.all(from i in "invites", where: like(i.id, ^(prefix <> "%")), select: i.id) do
      [id] ->
        Repo.update_all(from(i in "invites", where: i.id == ^id), set: [revoked_at: now()])
        {n, _} = Repo.delete_all(from m in "members", where: m.how == "invite" and m.ref == ^id)
        {:ok, if(n == 1, do: :member_removed, else: :unused)}

      [] -> {:error, :invite}
      _ -> {:error, :ambiguous}
    end
  end

  @doc "How many members, by the way they joined."
  def count, do: Repo.all(from m in "members", group_by: m.how, select: {m.how, count()}) |> Map.new()

  # ---- Helpers ------------------------------------------------------------------

  defp add!(pass, how, ref) do
    case Repo.insert_all("members", [%{pass: pass, how: how, ref: ref, added_at: now()}], on_conflict: :nothing) do
      {1, _} -> :ok
      {0, _} -> if Repo.in_transaction?(), do: Repo.rollback(:pass_taken), else: {:error, :pass_taken}
    end
  end

  defp pass_hash(hex), do: if(is_binary(hex) and hex =~ ~r/\A[0-9a-f]{64}\z/, do: {:ok, hex}, else: {:error, :pass})
  defp ok_or({:ok, v}, _), do: {:ok, v}
  defp ok_or(:error, what), do: {:error, what}
  defp hex(b), do: Base.encode16(b, case: :lower)
  defp now, do: DateTime.utc_now() |> DateTime.truncate(:second)
end
