defmodule Den.Host do
  @moduledoc """
  What the host runs on the den's server, against the running den:

      bin/den rpc 'Den.Host.invite(5, 7)'     # 5 one-use links, good for 7 days
      bin/den rpc 'Den.Host.invites()'        # made, used, withdrawn
      bin/den rpc 'Den.Host.withdraw("3fa9c1d2")'  # an invite, and the pass it let in
      bin/den rpc 'Den.Host.requests()'       # people asking to join
      bin/den rpc 'Den.Host.approve("…")'     # or Den.Host.decline("…")
      bin/den rpc 'Den.Host.members()'        # how many, by the way they joined
      bin/den rpc 'Den.Host.owner("a1b2c3")'  # the device named m-a1b2c3… owns the den
      bin/den rpc 'Den.Host.audit(30)'        # what moderators did

  An invite link carries its code after `#`, so the code never reaches a
  server log: whoever opens it on a device joins with that device.
  """
  import Ecto.Query, only: [from: 2]
  alias Den.Gate.Members

  def invite(n, days \\ 7)

  def invite(n, days) when not (is_integer(n) and n in 1..100 and is_integer(days) and days in 1..90) do
    IO.puts("epeaky invite N DAYS: N from 1 to 100 at a time, DAYS from 1 to 90")
    :error
  end

  def invite(n, days) do
    base = DenWeb.Endpoint.url()
    for code <- Members.invite(n, days), do: IO.puts("#{base}/#invite=#{code}")
    unless Members.open?(:invite), do: IO.puts("(invite links are not open on this den: add invite to DEN_ENTRY)")
    :ok
  end

  def invites do
    for i <- Members.invites() do
      state = cond do
        i.revoked_at -> "withdrawn"
        i.used_at -> "used #{i.used_at}"
        DateTime.compare(i.expires_at, DateTime.utc_now()) == :lt -> "expired"
        true -> "open until #{i.expires_at}"
      end

      IO.puts("#{String.slice(i.id, 0, 8)}  made #{i.made_at}  #{state}")
    end

    :ok
  end

  def withdraw(prefix), do: Members.withdraw(prefix)

  def requests do
    for r <- Members.pending(), do: IO.puts("#{r.id}  #{r.asked_at}  #{r.note}")
    :ok
  end

  def approve(id), do: Members.approve(id)
  def decline_all, do: IO.puts("declined #{Members.decline_all()} waiting requests")
  def decline(id), do: Members.decline(id)
  def members, do: Members.count()

  @doc """
  The den's owner (every permission, above every role), or make one: the
  device whose name (m- and hex, as the page shows it) starts with `prefix`,
  among the devices in the den today. At least six characters; more when
  several devices start alike.
  """
  def owner do
    case Den.Mod.owner() do
      nil -> IO.puts("This den has no owner. Make one: epeaky owner m-xxxxxx (your device's name in the page).")
      key -> IO.puts("The owner is m-#{String.slice(key, 0, 20)}")
    end
  end

  def owner(:clear) do
    :ok = Den.Mod.set_owner(nil)
    IO.puts("The den has no owner now. Roles stay; nobody can change them until there is one.")
  end

  def owner(prefix) when is_binary(prefix) do
    prefix = prefix |> String.downcase() |> String.replace_prefix("m-", "")
    now = DateTime.utc_now()

    cond do
      not (prefix =~ ~r/\A[0-9a-f]{6,64}\z/) ->
        IO.puts("Give the device's name as the page shows it: m- and at least six characters.")
        :error

      true ->
        case Den.Repo.all(from d in Den.Rooms.Device, where: like(d.key, ^(prefix <> "%")) and d.until > ^now, select: d.key) do
          [key] ->
            :ok = Den.Mod.set_owner(key)
            IO.puts("m-#{String.slice(key, 0, 20)} owns the den now. Den settings are in its page, under the den's name.")

          [] ->
            IO.puts("No device in the den today has a name starting m-#{prefix}. Open the den on that device, then run this again.")
            :error

          _ ->
            IO.puts("Several devices have names starting m-#{prefix}. Give more of the name.")
            :error
        end
    end
  end

  def audit(n \\ 30) when is_integer(n) do
    for a <- Enum.reverse(Den.Mod.audit(n)) do
      who = if a.actor, do: "m-" <> String.slice(a.actor, 0, 10), else: "host"
      what = if a.target, do: " " <> short(a.target), else: ""
      why = if a.reason, do: "  (#{a.reason})", else: ""
      IO.puts("#{a.at}  #{who}  #{a.action}#{what}#{why}")
    end

    :ok
  end

  defp short(<<_::binary-size(64)>> = key), do: "m-" <> String.slice(key, 0, 10)
  defp short(other), do: other

  @doc """
  A new den's first invite link, printed for the installer: only while the den
  has nobody and no invite, so running the installer again makes no more.
  """
  def first_invite do
    case Members.first_invite() do
      nil -> IO.puts("(no first invite: this den already has members or invites; make one with Den.Host.invite/2)")
      code -> IO.puts("#{DenWeb.Endpoint.url()}/#invite=#{code}")
    end
  end
end
