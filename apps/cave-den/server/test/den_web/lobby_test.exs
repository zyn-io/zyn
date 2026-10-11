defmodule DenWeb.LobbyTest do
  use DenWeb.ChannelCase, async: false
  import Phoenix.ConnTest, except: [connect: 2, connect: 3]
  import Plug.Conn, only: [put_req_header: 3]
  @f Path.expand("../fixtures", __DIR__)

  defp device(key) do
    Den.Rooms.admit(key, DateTime.add(DateTime.utc_now(), 3600, :second) |> DateTime.truncate(:second))
    s = Phoenix.Token.sign(DenWeb.Endpoint, "den", %{key: key})
    {:ok, socket} = connect(DenWeb.UserSocket, %{"session" => s})
    {socket, s}
  end

  setup do
    Den.Rooms.seed()
    :ok
  end

  test "the lobby lists open rooms, takes new ones (3 a day), never lists private ones" do
    {socket, _} = device(String.duplicate("a1", 32))
    {:ok, %{rooms: rooms}, socket} = subscribe_and_join(socket, DenWeb.LobbyChannel, "lobby")
    assert Enum.map(rooms, & &1.id) |> Enum.sort() == ~w(diggers general lanterns lookouts)
    ref = push(socket, "open_room", %{"title" => "  Night watch!  "})
    assert_reply ref, :ok, %{id: "r-night-watch-" <> _, title: "Night watch!"}
    assert_broadcast "room", %{title: "Night watch!", kind: "open"}
    for t <- ["b", "c"], do: (ref = push(socket, "open_room", %{"title" => t}); assert_reply(ref, :ok, _))
    ref = push(socket, "open_room", %{"title" => "d"})
    assert_reply ref, :error, %{reason: "A device can open 3 rooms a day. Try again tomorrow."}
    ref = push(socket, "private_room", %{"id" => "p-" <> String.duplicate("0f", 16)})
    assert_reply ref, :ok
    ref = push(socket, "private_room", %{"id" => "p-guessable"})
    assert_reply ref, :error, %{reason: "id"}
    refute Enum.any?(Den.Rooms.listed(), &(&1.kind == "private"))
    assert length(Den.Rooms.listed()) == 7
  end

  test "a private room refuses public joins and keeps no GroupInfo" do
    {socket, _} = device(String.duplicate("a2", 32))
    {:ok, _} = Den.Rooms.private_room("p-" <> String.duplicate("1e", 16))
    {:ok, reply, room} = subscribe_and_join(socket, DenWeb.RoomChannel, "room:p-" <> String.duplicate("1e", 16))
    assert reply.kind == "private" and reply.group_info == nil
    ref = push(room, "create", %{"group_info" => Base.encode64("info")})
    assert_reply ref, :error, %{reason: "exists"}
    # join.bin is an external commit (for "general"): refused here first for the room, and private rooms refuse any.
    ref = push(room, "relay", %{"bytes" => Base.encode64(File.read!(Path.join(@f, "join.bin")))})
    assert_reply ref, :error, _
  end

  test "invitations: published by a device, claimed once each, Welcomes waiting in its inbox only" do
    b = String.duplicate("b2", 32)
    {sb, _} = device(b)
    {sa, _} = device(String.duplicate("a3", 32))
    {:ok, %{welcomes: [], key_packages: 0}, inbox} = subscribe_and_join(sb, DenWeb.InboxChannel, "inbox:" <> b)
    assert {:error, %{reason: "not yours"}} = join(sa, DenWeb.InboxChannel, "inbox:" <> b)
    ref = push(inbox, "publish", %{"key_packages" => [Base.encode64("kp1"), Base.encode64("kp2")]})
    assert_reply ref, :ok, %{left: 2}
    {:ok, _, lobby} = subscribe_and_join(sa, DenWeb.LobbyChannel, "lobby")
    ref = push(lobby, "claim", %{"key" => b})
    assert_reply ref, :ok, %{key_package: kp}
    assert Base.decode64!(kp) == "kp1"
    ref = push(lobby, "claim", %{"key" => b})
    assert_reply ref, :ok, _
    ref = push(lobby, "claim", %{"key" => b})
    assert_reply ref, :error, %{reason: "That member's device has to come online once before it can be invited."}
    # A Welcome to B, through a room A is in.
    {:ok, _} = Den.Rooms.private_room("p-" <> String.duplicate("2d", 16))
    {:ok, _, room} = subscribe_and_join(sa, DenWeb.RoomChannel, "room:p-" <> String.duplicate("2d", 16))
    ref = push(room, "invite", %{"to" => [b], "welcome" => Base.encode64("welcome")})
    assert_reply ref, :ok
    assert_push "welcome", %{room: "p-" <> _, welcome: w}
    assert Base.decode64!(w) == "welcome"
    [%{id: id}] = Den.Inbox.waiting(b)
    ref = push(inbox, "taken", %{"ids" => [id]})
    assert_reply ref, :ok
    assert Den.Inbox.waiting(b) == []
  end

  test "typing is relayed, not kept" do
    {socket, _} = device(String.duplicate("a4", 32))
    {:ok, _, room} = subscribe_and_join(socket, DenWeb.RoomChannel, "room:general")
    push(room, "typing", %{})
    assert_broadcast "typing", %{key: _}
  end

  test "files: ciphertext in and out for admitted devices only" do
    {_, s} = device(String.duplicate("a5", 32))
    Application.put_env(:den, :blobs_dir, Path.join(System.tmp_dir!(), "den-blobs-test"))
    conn = build_conn() |> put_req_header("content-type", "application/octet-stream")
    assert conn |> post("/blob/", "x") |> json_response(401)
    up = conn |> put_req_header("authorization", "Bearer " <> s) |> post("/blob/", "sealed bytes") |> json_response(200)
    assert up["id"] == :crypto.hash(:sha256, "sealed bytes") |> Base.encode16(case: :lower)
    got = build_conn() |> put_req_header("authorization", "Bearer " <> s) |> get("/blob/" <> up["id"])
    assert got.status == 200 and got.resp_body == "sealed bytes"
    assert build_conn() |> get("/blob/" <> up["id"]) |> json_response(404)
  end

  test "each ZCaveDAO proposal gets a room, once" do
    Application.put_env(:den, :dao_check, fn -> {:ok, [1, 2]} end)
    DenWeb.Endpoint.subscribe("lobby")
    assert Den.Proposals.run() |> Enum.sort() == ["proposal-1", "proposal-2"]
    assert_receive %Phoenix.Socket.Broadcast{event: "room", payload: %{title: "Proposal #1"}}
    assert Den.Proposals.run() == []
    Application.delete_env(:den, :dao_check)
  end
end
