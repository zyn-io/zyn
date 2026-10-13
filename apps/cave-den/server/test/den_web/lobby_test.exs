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
    assert Enum.map(rooms, & &1.id) |> Enum.sort() == ~w(general news)
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
    assert length(Den.Rooms.listed()) == 5
  end

  test "a private room: tracked from its creator's GroupInfo, kept for nobody; no public joins; the add checked" do
    m = Path.join(@f, "mls")
    %{"a" => ka, "b" => kb} = Jason.decode!(File.read!(Path.join(m, "keys.json")))
    room = "p-00112233445566778899aabbccddeeff"
    {:ok, _} = Den.Rooms.private_room(room, ka)
    {sa, _} = device(ka)
    {sb, _} = device(kb)
    # Before the group exists, only the creator gets in; knowing the id is not enough.
    assert {:error, %{reason: "no such room"}} = join(sb, DenWeb.RoomChannel, "room:" <> room)
    {:ok, reply, a} = subscribe_and_join(sa, DenWeb.RoomChannel, "room:" <> room)
    assert reply.kind == "private" and reply.group_info == nil
    ref = push(a, "create", %{"group_info" => Base.encode64(File.read!(Path.join(m, "private.info")))})
    assert_reply ref, :ok
    assert Den.Rooms.get(room).group_info == nil and is_binary(Den.Rooms.get(room).public_state)
    assert Den.Rooms.members(Den.Rooms.get(room)) == [ka]
    # B, who holds no invitation, is not let into the channel or its voice,
    # and an external join (even from a member's socket) is refused.
    assert {:error, %{reason: "no such room"}} = join(sb, DenWeb.RoomChannel, "room:" <> room)
    assert {:error, %{reason: "no such room"}} = join(sb, DenWeb.VoiceChannel, "voice:" <> room)
    ref = push(a, "relay", %{"bytes" => Base.encode64(File.read!(Path.join(m, "private-join.bin")))})
    assert_reply ref, :error, %{reason: "a private room is joined by invitation"}
    # A's add of C: checked and ordered.
    ref = push(a, "relay", %{"bytes" => Base.encode64(File.read!(Path.join(m, "private-add.bin")))})
    assert_reply ref, :ok, %{epoch: 1}
    assert length(Den.Rooms.members(Den.Rooms.get(room))) == 2, "the den's member list follows the commit"
    # A member a commit takes out leaves the channel at once.
    Process.flag(:trap_exit, true)
    ref = Process.monitor(a.channel_pid)
    DenWeb.Endpoint.broadcast("room:" <> room, "members", %{keys: [kb]})
    assert_receive {:DOWN, ^ref, :process, _, {:shutdown, :removed}}, 2000
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
    {:ok, _} = Den.Rooms.private_room("p-" <> String.duplicate("2d", 16), String.duplicate("a3", 32))
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
    authed = put_req_header(conn, "authorization", "Bearer " <> s)
    assert authed |> post("/blob/", "sealed bytes") |> json_response(413), "no length sent first"
    assert authed |> put_req_header("content-length", "5") |> post("/blob/", "sealed bytes") |> json_response(413), "more than it said"
    assert authed |> put_req_header("content-length", "#{Den.Blobs.max() + 1}") |> post("/blob/", "x") |> json_response(413), "past the den's file size, refused unread"
    up = authed |> put_req_header("content-length", "12") |> post("/blob/", "sealed bytes") |> json_response(200)
    assert up["id"] == :crypto.hash(:sha256, "sealed bytes") |> Base.encode16(case: :lower)
    got = build_conn() |> put_req_header("authorization", "Bearer " <> s) |> get("/blob/" <> up["id"])
    assert got.status == 200 and got.resp_body == "sealed bytes"
    assert build_conn() |> get("/blob/" <> up["id"]) |> json_response(404)
  end


  describe "moderation in the lobby" do
    @owner String.duplicate("0a", 32)
    @mod String.duplicate("1b", 32)
    @member String.duplicate("3d", 32)

    setup do
      :ok = Den.Mod.set_owner(@owner)
      {o, _} = device(@owner)
      {m, _} = device(@mod)
      {x, _} = device(@member)
      {:ok, %{mod: %{mine: mine, owner: @owner}}, o} = subscribe_and_join(o, DenWeb.LobbyChannel, "lobby")
      assert "ban" in mine
      {:ok, %{mod: %{mine: []}}, m} = subscribe_and_join(m, DenWeb.LobbyChannel, "lobby")
      {:ok, _, x} = subscribe_and_join(x, DenWeb.LobbyChannel, "lobby")
      %{o: o, m: m, x: x}
    end

    test "the owner makes a role and gives it; everyone hears; the moderator then acts below it only", %{o: o, m: m, x: x} do
      ref = push(o, "role_save", %{"name" => "Moderator", "colour" => "#e0a030", "position" => 5, "hoist" => true, "perms" => ["timeout", "kick", "manage_channels"]})
      assert_reply ref, :ok
      assert_broadcast "mod", %{roles: roles}
      %{id: id} = Enum.find(roles, &(&1.name == "Moderator"))
      ref = push(o, "role_give", %{"key" => @mod, "id" => id})
      assert_reply ref, :ok
      assert_broadcast "mod", %{holders: %{@mod => [^id]}}

      ref = push(m, "timeout", %{"key" => @member, "seconds" => 600, "reason" => "flooding"})
      assert_reply ref, :ok
      assert_broadcast "mod", %{sanctions: [%{key: @member, kind: "timeout"}]}
      ref = push(m, "timeout", %{"key" => @owner, "seconds" => 600})
      assert_reply ref, :error, %{reason: "You can only do that to members and roles below your highest role."}
      ref = push(m, "ban", %{"key" => @member})
      assert_reply ref, :error, %{reason: "You do not have permission to do that."}
      ref = push(x, "kick", %{"key" => @mod})
      assert_reply ref, :error, %{reason: "You do not have permission to do that."}

      # Channels: made by Manage channels only, set up, deleted (not the host's own).
      ref = push(x, "open_room", %{"title" => "Mine"})
      assert_reply ref, :error, %{reason: "You do not have permission to do that."}
      ref = push(m, "open_room", %{"title" => "Announcements"})
      assert_reply ref, :ok, %{id: rid}
      ref = push(m, "room_save", %{"id" => rid, "slow" => 30, "topic" => "News from the team", "post_roles" => [id]})
      assert_reply ref, :ok, %{slow: 30, post_roles: [^id]}
      assert_broadcast "room", %{id: ^rid, slow: 30, topic: "News from the team"}
      ref = push(m, "room_delete", %{"id" => "general"})
      assert_reply ref, :error, %{reason: "This channel is set by the host; it can be renamed, not deleted."}
      ref = push(m, "room_delete", %{"id" => rid})
      assert_reply ref, :ok
      assert_broadcast "room_gone", %{id: ^rid}

      ref = push(m, "audit", %{})
      assert_reply ref, :ok, %{entries: [%{action: "channel deleted"} | _]}
      ref = push(x, "audit", %{})
      assert_reply ref, :error, _
    end

    test "a ban drops the device and tells the rooms; the owner sees the bans; settings reach everyone", %{o: o} do
      DenWeb.Endpoint.subscribe("device:" <> @member)
      ref = push(o, "ban", %{"key" => @member, "reason" => "spam bot"})
      assert_reply ref, :ok
      assert_receive %Phoenix.Socket.Broadcast{event: "disconnect"}
      ref = push(o, "bans", %{})
      assert_reply ref, :ok, %{bans: [%{key: @member, reason: "spam bot"}]}
      ref = push(o, "settings_save", %{"name" => "Zyn community", "description" => "Builders on Zyn"})
      assert_reply ref, :ok
      assert_broadcast "mod", %{den: %{name: "Zyn community"}}
    end

    test "Manage invites makes invite links and sees requests", %{o: o, m: m} do
      ref = push(m, "invite_make", %{"n" => 2, "days" => 7})
      assert_reply ref, :error, _
      ref = push(o, "invite_make", %{"n" => 2, "days" => 7})
      assert_reply ref, :ok, %{links: [l1, _]}
      assert l1 =~ ~r"/#invite=[0-9a-f]{64}$"
      ref = push(o, "invites", %{})
      assert_reply ref, :ok, %{invites: [_, _]}
      ref = push(o, "requests", %{})
      assert_reply ref, :ok, %{requests: []}
    end
  end
end
