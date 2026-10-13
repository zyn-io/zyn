defmodule DenWeb.LimitsTest do
  # What one device may put on the den in a day, and what one reply may carry.
  use DenWeb.ChannelCase, async: false

  @m Path.expand("../fixtures/mls", __DIR__)
  defp keys, do: Jason.decode!(File.read!(Path.join(@m, "keys.json")))

  defp device(key) do
    Den.Rooms.admit(key, DateTime.add(DateTime.utc_now(), 3600, :second) |> DateTime.truncate(:second))
    {:ok, socket} = connect(DenWeb.UserSocket, %{"session" => Phoenix.Token.sign(DenWeb.Endpoint, "den", %{key: key})})
    socket
  end

  setup do
    Den.Rooms.seed()
    on_exit(fn -> Application.delete_env(:den, :limits) end)
    :ok
  end

  test "a device's messages are capped in bytes a day" do
    %{"a" => ka, "b" => kb} = keys()
    {:ok, _, a} = subscribe_and_join(device(ka), DenWeb.RoomChannel, "room:general")
    {:ok, _, b} = subscribe_and_join(device(kb), DenWeb.RoomChannel, "room:general")
    ref = push(a, "create", %{"group_info" => Base.encode64(File.read!(Path.join(@m, "general.info")))})
    assert_reply ref, :ok
    join = File.read!(Path.join(@m, "join.bin"))
    text = File.read!(Path.join(@m, "text.bin"))
    # B may write its join and one message, not a second.
    Application.put_env(:den, :limits, %{message_bytes: byte_size(join) + byte_size(text)})
    ref = push(b, "relay", %{"bytes" => Base.encode64(join)})
    assert_reply ref, :ok
    ref = push(b, "relay", %{"bytes" => Base.encode64(text)})
    assert_reply ref, :ok
    ref = push(b, "relay", %{"bytes" => Base.encode64(text)})
    assert_reply ref, :error, %{reason: "this device has written enough for today"}
  end

  test "history comes in pages of at most :history_bytes, and says when there is more" do
    %{"a" => ka, "b" => kb} = keys()
    {:ok, _, a} = subscribe_and_join(device(ka), DenWeb.RoomChannel, "room:general")
    {:ok, _, b} = subscribe_and_join(device(kb), DenWeb.RoomChannel, "room:general")
    ref = push(a, "create", %{"group_info" => Base.encode64(File.read!(Path.join(@m, "general.info")))})
    assert_reply ref, :ok
    ref = push(b, "relay", %{"bytes" => Base.encode64(File.read!(Path.join(@m, "join.bin")))})
    assert_reply ref, :ok
    for _ <- 1..3 do
      ref = push(b, "relay", %{"bytes" => Base.encode64(File.read!(Path.join(@m, "text.bin")))})
      assert_reply ref, :ok
    end
    Application.put_env(:den, :limits, %{history_bytes: 1})
    ref = push(a, "history", %{"after" => 0})
    assert_reply ref, :ok, %{messages: [%{seq: 1}], more: true}, 1000
    ref = push(a, "history", %{"after" => 3})
    assert_reply ref, :ok, %{messages: [%{seq: 4}], more: false}, 1000
  end

  test "invitations are capped a day, and an inbox holds at most :inbox Welcomes" do
    room = "p-" <> String.duplicate("4d", 16)
    {:ok, _} = Den.Rooms.private_room(room)
    sender = String.duplicate("e1", 32)
    {:ok, _, r} = subscribe_and_join(device(sender), DenWeb.RoomChannel, "room:" <> room)
    victim = String.duplicate("e2", 32)
    Den.Rooms.admit(victim, DateTime.add(DateTime.utc_now(), 3600, :second) |> DateTime.truncate(:second))
    Application.put_env(:den, :limits, %{inbox: 2, invites: 3})
    for _ <- 1..3 do
      ref = push(r, "invite", %{"to" => [victim], "welcome" => Base.encode64("w")})
      assert_reply ref, :ok
    end
    assert length(Den.Inbox.waiting(victim)) == 2, "the inbox keeps no more than its cap"
    ref = push(r, "invite", %{"to" => [victim], "welcome" => Base.encode64("w")})
    assert_reply ref, :error, %{reason: "quota"}
  end

  test "a Welcome goes only to devices the den knows, and every copy counts against the sender's bytes" do
    room = "p-" <> String.duplicate("5e", 16)
    {:ok, _} = Den.Rooms.private_room(room)
    sender = String.duplicate("e3", 32)
    {:ok, _, r} = subscribe_and_join(device(sender), DenWeb.RoomChannel, "room:" <> room)
    known = String.duplicate("e4", 32)
    Den.Rooms.admit(known, DateTime.add(DateTime.utc_now(), 3600, :second) |> DateTime.truncate(:second))
    strangers = for i <- 1..40, do: :crypto.hash(:sha256, <<i>>) |> Base.encode16(case: :lower)
    Application.put_env(:den, :limits, %{message_bytes: 25_000})
    big = Base.encode64(:binary.copy(<<7>>, 12_000))
    ref = push(r, "invite", %{"to" => [known | strangers], "welcome" => big})
    assert_reply ref, :ok
    assert length(Den.Inbox.waiting(known)) == 1
    assert Enum.all?(strangers, &(Den.Inbox.waiting(&1) == [])), "nothing is stored for keys the den never admitted"
    ref = push(r, "invite", %{"to" => [known, known], "welcome" => big})
    assert_reply ref, :ok
    ref = push(r, "invite", %{"to" => [known], "welcome" => big})
    # 12 KB a copy, the same device once: the third copy is past 25 KB.
    assert_reply ref, :error, %{reason: "quota"}
  end

  test "below the free-disk floor, nothing is taken" do
    Application.put_env(:den, :limits, %{min_free_bytes: 1_000_000_000_000_000})
    :persistent_term.erase({Den.Limits, :disk})
    refute Den.Limits.disk_ok?()
    Application.delete_env(:den, :limits)
    :persistent_term.erase({Den.Limits, :disk})
    assert Den.Limits.disk_ok?()
  end
end
