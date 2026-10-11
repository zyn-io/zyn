defmodule Den.Ledger do
  @moduledoc """
  The object ledger's word on a pass: `zyn_memberPass`, live or not. The only
  question the den ever asks the ledger.
  """

  @doc "Whether `pass` is a live member pass of the collection. {:ok, bool} | {:error, reason}"
  def live?(pass) do
    case Application.get_env(:den, :ledger_check) do
      fun when is_function(fun, 1) -> fun.(pass)
      _ -> ask(pass)
    end
  end

  defp ask(pass) do
    node = Application.fetch_env!(:den, :node_url)
    collection = Application.fetch_env!(:den, :collection)
    body = Jason.encode!(%{jsonrpc: "2.0", id: 1, method: "zyn_memberPass", params: %{collection: collection, pass: Base.encode16(pass, case: :lower)}})

    case :httpc.request(:post, {String.to_charlist(node), [], ~c"application/json", body}, [timeout: 10_000], body_format: :binary) do
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
