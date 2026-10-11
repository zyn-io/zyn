defmodule DenWeb.RoomChannelTest do
  use DenWeb.ChannelCase, async: false

  @f Path.expand("../fixtures", __DIR__)
  defp b64(f), do: Base.encode64(File.read!(Path.join(@f, f)))

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

  test "a room: made once, one commit per epoch, words in order, history", %{socket: socket, key: key} do
    {:ok, reply, socket} = subscribe_and_join(socket, DenWeb.RoomChannel, "room:general")
    assert %{epoch: 0, seq: 0, group_info: nil, title: "General"} = reply
    assert reply.present == [key]
    assert {:error, %{reason: "no such room"}} = join(socket, DenWeb.RoomChannel, "room:nowhere")

    # Words before any group are refused; the first create wins.
    ref = push(socket, "relay", %{"bytes" => b64("text.bin")})
    assert_reply ref, :error, %{reason: "no group yet"}
    ref = push(socket, "create", %{"group_info" => Base.encode64("info-0")})
    assert_reply ref, :ok
    ref = push(socket, "create", %{"group_info" => Base.encode64("other")})
    assert_reply ref, :error, %{reason: "exists"}

    # The external join at epoch 0 moves the room to 1; a second commit at 0 is refused.
    ref = push(socket, "relay", %{"bytes" => b64("join.bin")})
    assert_reply ref, :ok, %{seq: 1, epoch: 1}
    ref = push(socket, "relay", %{"bytes" => b64("join.bin")})
    assert_reply ref, :error, %{reason: "epoch", epoch: 1}

    # Words at epoch 1; a commit for epoch 2 is early; GroupInfo for the right epoch only.
    ref = push(socket, "relay", %{"bytes" => b64("text.bin")})
    assert_reply ref, :ok, %{seq: 2, epoch: 1}
    ref = push(socket, "relay", %{"bytes" => b64("remove.bin")})
    assert_reply ref, :error, %{reason: "epoch", epoch: 1}
    ref = push(socket, "info", %{"epoch" => 0, "group_info" => Base.encode64("stale")})
    assert_reply ref, :error, %{reason: "stale"}
    ref = push(socket, "info", %{"epoch" => 1, "group_info" => Base.encode64("info-1")})
    assert_reply ref, :ok
    assert Den.Rooms.get("general").group_info == "info-1"

    # A message for another room is not relayed here.
    {:ok, _, other} = subscribe_and_join(socket, DenWeb.RoomChannel, "room:lanterns")
    ref = push(other, "create", %{"group_info" => Base.encode64("x")})
    assert_reply ref, :ok
    ref = push(other, "relay", %{"bytes" => b64("text.bin")})
    assert_reply ref, :error, %{reason: ":wrong_room"}

    ref = push(socket, "state", %{})
    assert_reply ref, :ok, %{epoch: 1, seq: 2, group_info: "aW5mby0x"}
    ref = push(socket, "history", %{"after" => 0})
    assert_reply ref, :ok, %{messages: [%{seq: 1, epoch: 0, commit: true}, %{seq: 2, epoch: 1, commit: false}]}
    ref = push(socket, "history", %{"after" => 1})
    assert_reply ref, :ok, %{messages: [%{seq: 2}]}
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
