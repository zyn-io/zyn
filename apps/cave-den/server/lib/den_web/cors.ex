defmodule DenWeb.Cors do
  @moduledoc "The door answers the den's own page and the origins in `:origins`, and nobody else's."
  import Plug.Conn

  def init(opts), do: opts

  def call(conn, _) do
    origin = get_req_header(conn, "origin") |> List.first()
    allowed = Application.get_env(:den, :origins, [])

    conn =
      if origin in allowed do
        conn
        |> put_resp_header("access-control-allow-origin", origin)
        |> put_resp_header("access-control-allow-methods", "GET, POST, OPTIONS")
        |> put_resp_header("access-control-allow-headers", "content-type, authorization")
        |> put_resp_header("vary", "origin")
      else
        conn
      end

    if conn.method == "OPTIONS", do: conn |> send_resp(204, "") |> halt(), else: conn
  end
end
