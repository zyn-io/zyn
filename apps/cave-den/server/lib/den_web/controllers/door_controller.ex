defmodule DenWeb.DoorController do
  use DenWeb, :controller

  @reasons %{
    code: {400, "send the member pass code Nap gave you"},
    day: {400, "a token is for today or tomorrow, and is used on its day"},
    blinded: {400, "that is not a blinded token"},
    not_live: {403, "that pass is not live: get one on /dao with a Collection Z item in your Nap"},
    limit: {429, "this pass has had its tokens for that day"},
    ledger: {503, "the ledger is not answering — try again shortly"},
    no_key: {503, "the door has no key for that day yet"},
    token: {400, "that is not a den token"},
    signature: {403, "that token was not signed by the door for its day"},
    spent: {403, "that token was already used"},
    proof: {403, "this device did not prove it holds the key its token names"}
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
end
