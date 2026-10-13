defmodule Den.Gate.Zyn do
  @moduledoc """
  The Cave's gate (DECISIONS §184): a member pass is live while its holder
  holds a Collection Z item, on Zyn's object ledger (`zyn_memberPass`). The
  pass is the ledger's `objectvm::dao::pass_of`: SHA-256 over a leaf prefix,
  the domain and the code Nap made. Pass hashes are public on ZynScan, so the
  door asks for the code and hashes it itself.

  Compiled into the den from `extra/` (see apps/cave-den/cave/build-server.sh),
  set with `DEN_GATE=Den.Gate.Zyn`; reads `DEN_NODE_URL` and `DEN_COLLECTION`.
  """
  @behaviour Den.Gate

  @domain "objectvm.dao-pass.v1"

  @impl true
  def id(<<code::binary-size(32)>>), do: :crypto.hash(:sha256, <<0>> <> @domain <> code)

  @impl true
  def live?(code) do
    case Application.get_env(:den, :zyn_check) do
      fun when is_function(fun, 1) -> fun.(id(code))
      _ -> ask(id(code))
    end
  end

  def node_url, do: Application.get_env(:den, :node_url) || System.get_env("DEN_NODE_URL", "http://127.0.0.1:8599")
  def collection, do: Application.get_env(:den, :collection) || System.fetch_env!("DEN_COLLECTION")

  defp ask(pass) do
    body = Jason.encode!(%{jsonrpc: "2.0", id: 1, method: "zyn_memberPass", params: %{collection: collection(), pass: Base.encode16(pass, case: :lower)}})

    case :httpc.request(:post, {String.to_charlist(node_url()), [], ~c"application/json", body}, [timeout: 10_000], body_format: :binary) do
      {:ok, {{_, 200, _}, _, resp}} ->
        case Jason.decode(resp) do
          {:ok, %{"result" => %{"member" => m}}} when is_boolean(m) -> {:ok, m}
          _ -> {:error, :bad_answer}
        end

      other ->
        {:error, {:ledger, other}}
    end
  end
end
