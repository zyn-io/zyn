defmodule Den.Cave.Proposals do
  @moduledoc """
  Every ZCaveDAO proposal gets a room: the den reads the DAO's public
  proposal list from the ledger (`zyn_dao`) every minute and opens
  `proposal-<n>` for each one it has not seen, announcing it to the lobby.
  Compiled into the den from `extra/`, started with
  `DEN_EXTRA_CHILDREN=Den.Cave.Proposals`.
  """
  use GenServer

  def start_link(_), do: GenServer.start_link(__MODULE__, nil, name: __MODULE__)

  @impl true
  def init(_) do
    Process.send_after(self(), :poll, 5_000)
    {:ok, nil}
  end

  @impl true
  def handle_info(:poll, s) do
    run()
    Process.send_after(self(), :poll, Application.get_env(:den, :proposals_ms, 60_000))
    {:noreply, s}
  end

  def run do
    with {:ok, ids} <- proposals() do
      for n <- ids, {:ok, room} <- [Den.Rooms.proposal_room(n)] do
        DenWeb.Endpoint.broadcast("lobby", "room", %{id: room.id, title: room.title, kind: room.kind})
        room.id
      end
    end
  end

  defp proposals do
    case Application.get_env(:den, :dao_check) do
      fun when is_function(fun, 0) -> fun.()
      _ -> ask()
    end
  end

  defp ask do
    node = Den.Gate.Zyn.node_url()
    body = Jason.encode!(%{jsonrpc: "2.0", id: 1, method: "zyn_dao", params: %{collection: Den.Gate.Zyn.collection()}})

    case :httpc.request(:post, {String.to_charlist(node), [], ~c"application/json", body}, [timeout: 10_000], body_format: :binary) do
      {:ok, {{_, 200, _}, _, resp}} ->
        case Jason.decode(resp) do
          {:ok, %{"result" => %{"proposals" => ps}}} when is_list(ps) -> {:ok, for(%{"id" => n} <- ps, is_integer(n), do: n)}
          {:ok, %{"result" => _}} -> {:ok, []}
          _ -> {:error, :bad_answer}
        end

      _ ->
        {:error, :ledger}
    end
  end
end
