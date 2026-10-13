defmodule DenWeb.DoorController do
  use DenWeb, :controller

  @reasons %{
    code: {400, "send a member pass code"},
    day: {400, "a token is for today or tomorrow, and is used on its day"},
    blinded: {400, "that is not a blinded token"},
    not_live: {403, "that pass is not a member of this den"},
    limit: {429, "this pass has had its tokens for that day"},
    gate: {503, "the den cannot check passes right now — try again shortly"},
    no_key: {503, "the door has no key for that day yet"},
    early: {425, "the door signs tomorrow's tokens only late in the day"},
    token: {400, "that is not a den token"},
    signature: {403, "that token was not signed by the door for its day"},
    spent: {403, "that token was already used"},
    proof: {403, "this device did not prove it holds the key its token names"},
    closed: {404, "this den does not let people in that way"},
    invite: {403, "that invite is not good: it was used, withdrawn or has expired"},
    pass: {400, "send the hash of this device's pass code"},
    pass_taken: {409, "that pass is already a member"},
    note: {400, "write a short note for the host (up to 280 characters)"},
    busy: {429, "too many people are waiting for the host; try again later"},
    request: {404, "there is no such request"},
    issuer: {401, "only the host's own site may register passes"}
  }

  defp fail(conn, why) do
    {code, text} = Map.get(@reasons, why, {400, "refused"})
    conn |> put_status(code) |> json(%{error: text})
  end

  def keys(conn, _), do: json(conn, %{keys: Den.Keys.published(Den.Door.today(), 31)})

  def sign(conn, %{"code" => code, "day" => day, "blinded" => blinded}) do
    case Den.Door.sign(code, day, blinded) do
      {:ok, sig} -> json(conn, %{blind_sig: Base.encode16(sig, case: :lower)})
      {:error, why} -> fail(conn, why)
    end
  end

  def sign(conn, _), do: fail(conn, :code)

  def enter(conn, %{"msg" => msg, "randomizer" => r, "sig" => s} = params) do
    case Den.Door.enter(msg, r, s, params["proof"]) do
      {:ok, session} -> json(conn, session)
      {:error, why} -> fail(conn, why)
    end
  end

  def enter(conn, _), do: fail(conn, :token)

  alias Den.Gate.Members

  def ways(conn, _) do
    members = Den.Gate.module() == Members
    json(conn, %{invite: members and Members.open?(:invite), approval: members and Members.open?(:approval), tomorrow_from: Den.Door.tomorrow_from(), file_max: Den.Blobs.max()})
  end

  def join(conn, %{"invite" => invite, "pass" => pass}), do: done(conn, Members.join(invite, pass))
  def join(conn, _), do: fail(conn, :invite)

  def ask(conn, %{"pass" => pass} = p) do
    case Members.ask(pass, p["note"], DenWeb.RateLimit.tag(conn.assigns[:client] || conn.remote_ip)) do
      {:ok, id} -> json(conn, %{id: id})
      {:error, why} -> fail(conn, why)
    end
  end

  def asked(conn, %{"id" => id}) do
    case Members.status(id) do
      {:ok, status} -> json(conn, %{status: status})
      {:error, why} -> fail(conn, why)
    end
  end

  def register(conn, params), do: issuer(conn, fn -> Members.register(params["pass"]) end)
  def remove(conn, %{"pass" => pass}), do: issuer(conn, fn -> Members.remove(pass) end)

  defp issuer(conn, f) do
    token = case get_req_header(conn, "authorization") do
      ["Bearer " <> t] -> t
      _ -> nil
    end

    if Members.issuer?(token), do: done(conn, f.()), else: fail(conn, :issuer)
  end

  defp done(conn, :ok), do: json(conn, %{ok: true})
  defp done(conn, {:error, why}), do: fail(conn, why)
end
