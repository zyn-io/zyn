defmodule Den.Host do
  @moduledoc """
  What the host runs on the den's server, against the running den:

      bin/den rpc 'Den.Host.invite(5, 7)'     # 5 one-use links, good for 7 days
      bin/den rpc 'Den.Host.invites()'        # made, used, withdrawn
      bin/den rpc 'Den.Host.withdraw("3fa9c1d2")'  # an invite, and the pass it let in
      bin/den rpc 'Den.Host.requests()'       # people asking to join
      bin/den rpc 'Den.Host.approve("…")'     # or Den.Host.decline("…")
      bin/den rpc 'Den.Host.members()'        # how many, by the way they joined

  An invite link carries its code after `#`, so the code never reaches a
  server log: whoever opens it on a device joins with that device.
  """
  alias Den.Gate.Members

  def invite(n, days \\ 7) do
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
  def decline(id), do: Members.decline(id)
  def members, do: Members.count()

  @doc "At boot: a new den prints its first invite link, once, for the host."
  def first_run do
    require Logger

    case Application.get_env(:den, :first_run, true) && Members.first_invite() do
      x when x in [nil, false] -> :ok
      code -> Logger.notice("Epeaky: this den has no members yet. Your first invite (one use, 30 days): #{DenWeb.Endpoint.url()}/#invite=#{code}")
    end
  end
end
