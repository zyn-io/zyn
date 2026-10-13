defmodule DenWeb.RateLimit do
  @moduledoc """
  The door's per-address limits, in fixed one-minute windows: every request
  to /door counts against `:door_per_minute` (60), and asking to join against
  `:ask_per_hour` (5). An address is the peer's, or, when the peer is a proxy
  on this machine (Caddy, nginx), the last address in X-Forwarded-For, which
  that proxy wrote.
  """
  @behaviour Plug
  import Plug.Conn

  @table __MODULE__

  def setup, do: :ets.new(@table, [:named_table, :public, :set, write_concurrency: true])

  @impl true
  def init(opts), do: opts

  @impl true
  def call(conn, _) do
    ip = client(conn)

    cond do
      over?({:door, ip}, 60, Application.get_env(:den, :door_per_minute, 60)) -> refuse(conn)
      conn.method == "POST" and conn.path_info == ["door", "ask"] and over?({:ask, ip}, 3600, Application.get_env(:den, :ask_per_hour, 5)) -> refuse(conn)
      true -> assign(conn, :client, ip)
    end
  end

  @doc "The address a request comes from, as above."
  def client(conn) do
    case {conn.remote_ip, get_req_header(conn, "x-forwarded-for")} do
      {ip, [fwd | _]} when ip in [{127, 0, 0, 1}, {0, 0, 0, 0, 0, 0, 0, 1}] ->
        last = fwd |> String.split(",") |> List.last() |> String.trim()
        case :inet.parse_address(String.to_charlist(last)) do
          {:ok, a} -> a
          _ -> ip
        end

      {ip, _} -> ip
    end
  end

  @doc "A keyed tag of an address: the same address gives the same tag, and the tag gives no address back."
  def tag(ip), do: :crypto.mac(:hmac, :sha256, DenWeb.Endpoint.config(:secret_key_base), :erlang.term_to_binary(ip)) |> binary_part(0, 16) |> Base.encode16(case: :lower)

  defp over?(key, window, max) do
    slot = div(System.system_time(:second), window)
    n = :ets.update_counter(@table, {key, slot}, 1, {{key, slot}, 0})
    if n == 1, do: :ets.select_delete(@table, [{{{key, :"$1"}, :_}, [{:<, :"$1", slot}], [true]}])
    n > max
  end

  defp refuse(conn) do
    conn |> put_status(429) |> Phoenix.Controller.json(%{error: "too many requests from this address; wait a minute"}) |> halt()
  end
end
