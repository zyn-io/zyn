defmodule DenWeb.RoomChannelTest do
  use DenWeb.ChannelCase, async: false

  @f Path.expand("../fixtures", __DIR__)

  setup do
    Den.Rooms.seed()
    key = String.duplicate("cd", 32)
    Den.Rooms.admit(key, DateTime.add(DateTime.utc_now(), 3600, :second) |> DateTime.truncate(:second))
    session = Phoenix.Token.sign(DenWeb.Endpoint, "den", %{key: key})
    {:ok, socket} = connect(DenWeb.UserSocket, %{"session" => session})
    %{socket: socket, key: key}
  end

  test "a device without a session, or whose day ended, cannot connect", %{key: key} do
    assert connect(DenWeb.UserSocket, %{}) == :error
    assert connect(DenWeb.UserSocket, %{"session" => "forged"}) == :error
    Den.Rooms.admit(key, DateTime.add(DateTime.utc_now(), -1, :second) |> DateTime.truncate(:second))
    assert connect(DenWeb.UserSocket, %{"session" => Phoenix.Token.sign(DenWeb.Endpoint, "den", %{key: key})}) == :error
  end

  # Real MLS messages from the den's library (test/fixtures/mls, written by
  # `cargo test write_server_fixtures -- --ignored`): A starts General, B joins
  # by external commit and writes, A removes B; X's "general" is a fork.
  @m Path.join(@f, "mls")
  defp mls(f), do: Base.encode64(File.read!(Path.join(@m, f)))
  defp keys, do: Jason.decode!(File.read!(Path.join(@m, "keys.json")))

  defp device(key) do
    Den.Rooms.admit(key, DateTime.add(DateTime.utc_now(), 3600, :second) |> DateTime.truncate(:second))
    {:ok, socket} = connect(DenWeb.UserSocket, %{"session" => Phoenix.Token.sign(DenWeb.Endpoint, "den", %{key: key})})
    socket
  end

  test "a room: made once, each commit checked and signed by its sender, words in order, history" do
    %{"a" => ka, "b" => kb} = keys()
    {:ok, reply, a} = subscribe_and_join(device(ka), DenWeb.RoomChannel, "room:general")
    assert %{epoch: 0, seq: 0, group_info: nil, title: "General"} = reply
    {:ok, _, b} = subscribe_and_join(device(kb), DenWeb.RoomChannel, "room:general")

    # Words before any group are refused; a GroupInfo must be a real one, for this room; the first wins.
    ref = push(b, "relay", %{"bytes" => mls("text.bin")})
    assert_reply ref, :error, %{reason: "no group yet"}
    ref = push(a, "create", %{"group_info" => Base.encode64("info-0")})
    assert_reply ref, :error, %{reason: "group_info"}
    ref = push(a, "create", %{"group_info" => mls("general.info")})
    assert_reply ref, :ok
    ref = push(a, "create", %{"group_info" => mls("general.info")})
    assert_reply ref, :error, %{reason: "exists"}

    # B's external join: refused when another device sends it, ordered when B does; not twice.
    ref = push(a, "relay", %{"bytes" => mls("join.bin")})
    assert_reply ref, :error, %{reason: "a commit is signed by the device that sends it"}
    ref = push(b, "relay", %{"bytes" => mls("join.bin")})
    assert_reply ref, :ok, %{seq: 1, epoch: 1}
    ref = push(b, "relay", %{"bytes" => mls("join.bin")})
    assert_reply ref, :error, %{reason: "epoch", epoch: 1}

    # Bytes shaped like a commit at the room's epoch no longer move it.
    junk = <<1::16, 2::16, 7, "general", 1::64, 3>>
    ref = push(a, "relay", %{"bytes" => Base.encode64(junk)})
    assert_reply ref, :error, %{reason: "that commit does not check out against the room"}
    assert Den.Rooms.get("general").epoch == 1

    # Words at epoch 1; a GroupInfo only if it is the room's own state.
    ref = push(b, "relay", %{"bytes" => mls("text.bin")})
    assert_reply ref, :ok, %{seq: 2, epoch: 1}
    ref = push(a, "info", %{"epoch" => 1, "group_info" => mls("fork.info")})
    assert_reply ref, :error, %{reason: "group_info"}
    ref = push(a, "info", %{"epoch" => 0, "group_info" => mls("general.info")})
    assert_reply ref, :error, %{reason: "stale"}
    ref = push(b, "info", %{"epoch" => 1, "group_info" => mls("general-1.info")})
    assert_reply ref, :ok
    assert Den.Rooms.get("general").group_info == File.read!(Path.join(@m, "general-1.info"))

    # A's commit removing B: not B's to send; A's.
    ref = push(b, "relay", %{"bytes" => mls("remove.bin")})
    assert_reply ref, :error, %{reason: "a commit is signed by the device that sends it"}
    ref = push(a, "relay", %{"bytes" => mls("remove.bin")})
    assert_reply ref, :ok, %{seq: 3, epoch: 2}

    # B, removed, still writing at epoch 1 (within the den's window for late
    # words): refused, so it cannot speak from a seat a newcomer will take.
    ref = push(b, "relay", %{"bytes" => mls("text.bin")})
    assert_reply ref, :error, %{reason: "this device is not a member of the room"}

    # Another room's GroupInfo, or messages, are not this room's.
    {:ok, _, other} = subscribe_and_join(a, DenWeb.RoomChannel, "room:news")
    ref = push(other, "create", %{"group_info" => mls("general.info")})
    assert_reply ref, :error, %{reason: "group_info"}
    ref = push(other, "relay", %{"bytes" => mls("text.bin")})
    assert_reply ref, :error, %{reason: ":wrong_room"}

    ref = push(a, "history", %{"after" => 0})
    assert_reply ref, :ok, %{messages: [%{seq: 1, epoch: 0, commit: true}, %{seq: 2, epoch: 1, commit: false}, %{seq: 3, epoch: 1, commit: true}]}
  end

  test "when a device's day ends, its rooms are told and its connection closed", %{socket: socket} do
    other = String.duplicate("ef", 32)
    Den.Rooms.admit(other, DateTime.add(DateTime.utc_now(), 3600, :second) |> DateTime.truncate(:second))
    Den.Rooms.seat("general", other)
    {:ok, reply, _socket} = subscribe_and_join(socket, DenWeb.RoomChannel, "room:general")
    assert other in reply.present
    Den.Rooms.admit(other, DateTime.add(DateTime.utc_now(), -5, :second) |> DateTime.truncate(:second))
    DenWeb.Endpoint.subscribe("device:" <> other)
    {by_room, ended} = Den.Sweeper.run()
    assert by_room == %{"general" => [other]} and ended == [other]
    assert_push "expired", %{keys: [^other]}
    assert_receive %Phoenix.Socket.Broadcast{event: "disconnect", topic: "device:" <> ^other}
    refute other in Den.Rooms.present("general")
  end
end
