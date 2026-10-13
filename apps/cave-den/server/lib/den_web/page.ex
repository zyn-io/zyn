defmodule DenWeb.Page do
  @moduledoc """
  The den's own page (`priv/page`, built by `web/build.sh`): "/" is its
  index.html, and its files go out under a policy that runs only the page's
  own scripts and wasm, reaches only this den and the origins in
  `:page_connect` (where the day keys are published, say), and allows the
  microphone for voice. No extension or third-party script runs where the
  keys are kept.
  """
  @behaviour Plug
  import Plug.Conn

  @api ~w(door blob socket)

  @impl true
  def init(_), do: Plug.Static.init(at: "/", from: {:den, "priv/page"}, gzip: false)

  @impl true
  def call(%Plug.Conn{path_info: [first | _]} = conn, _) when first in @api, do: conn

  def call(%Plug.Conn{path_info: []} = conn, _) do
    index = Application.app_dir(:den, "priv/page/index.html")

    if conn.method in ["GET", "HEAD"] and File.regular?(index) do
      conn
      |> secure()
      |> put_resp_content_type("text/html")
      |> put_resp_header("cache-control", "no-cache")
      |> send_file(200, index)
      |> halt()
    else
      conn
    end
  end

  def call(conn, static), do: Plug.Static.call(secure(conn), static)

  @doc "The page's policy, for this den's host and the extra origins it may reach."
  def csp(host, extra) do
    ws = if host in ["localhost", "127.0.0.1"], do: "ws://#{host}:*", else: "wss://#{host}"

    Enum.join(
      [
        "default-src 'none'",
        "script-src 'self' 'wasm-unsafe-eval'",
        "worker-src 'self'",
        "style-src 'self'",
        "img-src 'self' data: blob:",
        "media-src 'self' blob:",
        "font-src 'self'",
        "connect-src " <> Enum.join(["'self'", ws | extra], " "),
        "base-uri 'none'",
        "form-action 'none'",
        "frame-ancestors 'none'",
        "object-src 'none'"
      ],
      "; "
    )
  end

  defp secure(conn) do
    conn
    |> put_resp_header("content-security-policy", csp(conn.host, Application.get_env(:den, :page_connect, [])))
    |> put_resp_header("permissions-policy", "geolocation=(), microphone=(self), camera=(), payment=()")
    |> put_resp_header("x-frame-options", "DENY")
    |> put_resp_header("x-content-type-options", "nosniff")
    |> put_resp_header("referrer-policy", "no-referrer")
    # The way in is a window this page opens (gate.js); same-origin would cut it off.
    |> put_resp_header("cross-origin-opener-policy", "same-origin-allow-popups")
  end
end
