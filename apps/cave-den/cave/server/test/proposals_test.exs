defmodule Den.Cave.ProposalsTest do
  use DenWeb.ChannelCase, async: false

  test "each ZCaveDAO proposal gets a room, once" do
    Application.put_env(:den, :dao_check, fn -> {:ok, [1, 2]} end)
    DenWeb.Endpoint.subscribe("lobby")
    assert Den.Cave.Proposals.run() |> Enum.sort() == ["proposal-1", "proposal-2"]
    assert_receive %Phoenix.Socket.Broadcast{event: "room", payload: %{title: "Proposal #1"}}
    assert Den.Cave.Proposals.run() == []
    Application.delete_env(:den, :dao_check)
  end
end
