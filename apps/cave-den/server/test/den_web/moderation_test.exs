defmodule DenWeb.ModerationTest do
  use DenWeb.ChannelCase, async: false
  alias Den.Mod

  # Real MLS messages (test/fixtures/mls): A starts General, B joins and writes
  # (text.bin, epoch 1), B refreshes its keys (update.bin, epoch 1).
  @m Path.expand("../fixtures/mls", __DIR__)
  defp mls(f), do: Base.encode64(File.read!(Path.join(@m, f)))
  defp keys, do: Jason.decode!(File.read!(Path.join(@m, "keys.json")))
  @owner String.duplicate("0a", 32)

  defp session(key), do: Phoenix.Token.sign(DenWeb.Endpoint, "den", %{key: key})

  defp device(key) do
    Den.Rooms.admit(key, DateTime.add(DateTime.utc_now(), 3600, :second) |> DateTime.truncate(:second))
    {:ok, socket} = connect(DenWeb.UserSocket, %{"session" => session(key)})
    socket
  end

  setup do
    Den.Rooms.seed()
    :ok = Mod.set_owner(@owner)
    %{"a" => ka, "b" => kb} = keys()
    {:ok, _, a} = subscribe_and_join(device(ka), DenWeb.RoomChannel, "room:general")
    {:ok, _, b} = subscribe_and_join(device(kb), DenWeb.RoomChannel, "room:general")
    ref = push(a, "create", %{"group_info" => mls("general.info")})
    assert_reply ref, :ok
    ref = push(b, "relay", %{"bytes" => mls("join.bin")})
    assert_reply ref, :ok, %{epoch: 1}
    %{a: a, b: b, ka: ka, kb: kb}
  end

  test "a timed-out device's words are refused (its own commits are not); typing and voice too", %{a: a, b: b, kb: kb} do
    :ok = Mod.timeout(@owner, kb, 600, "flooding")
    ref = push(b, "relay", %{"bytes" => mls("text.bin")})
    assert_reply ref, :error, %{code: "timeout", until: until}
    assert until > System.os_time(:second)
    ref = push(b, "relay", %{"bytes" => mls("update.bin")})
    assert_reply ref, :ok, %{epoch: 2}
    push(b, "typing", %{})
    refute_broadcast "typing", %{}, 100
    assert {:error, %{reason: "You are timed out" <> _}} = subscribe_and_join(device(kb), DenWeb.VoiceChannel, "voice:general")
    :ok = Mod.untimeout(@owner, kb)
    push(b, "typing", %{})
    assert_broadcast "typing", %{key: ^kb}
    _ = a
  end

  test "who may post: a room for some roles only", %{b: b, kb: kb} do
    {:ok, team} = Mod.role_save(@owner, %{"name" => "Team", "position" => 2, "perms" => []})
    {:ok, _} = Mod.room_save(@owner, "general", %{"post_roles" => [team.id]})
    ref = push(b, "relay", %{"bytes" => mls("text.bin")})
    assert_reply ref, :error, %{code: "no_post", reason: "You do not have permission to send messages in this channel."}
    :ok = Mod.role_give(@owner, kb, team.id)
    ref = push(b, "relay", %{"bytes" => mls("text.bin")})
    assert_reply ref, :ok
  end

  test "slow mode: a few words a window, then refused; Bypass slow mode skips it", %{b: b, kb: kb} do
    {:ok, _} = Mod.room_save(@owner, "general", %{"slow" => 30})
    for _ <- 1..4 do
      ref = push(b, "relay", %{"bytes" => mls("text.bin")})
      assert_reply ref, :ok
    end
    ref = push(b, "relay", %{"bytes" => mls("text.bin")})
    assert_reply ref, :error, %{code: "slow", slow: 30, reason: "Slow mode is on: one message every 30 seconds."}
    {:ok, fast} = Mod.role_save(@owner, %{"name" => "Fast", "position" => 2, "perms" => ["bypass_slow"]})
    :ok = Mod.role_give(@owner, kb, fast.id)
    ref = push(b, "relay", %{"bytes" => mls("text.bin")})
    assert_reply ref, :ok
  end

  test "every message carries the den's time, live and in history", %{a: a, b: b} do
    ref = push(b, "relay", %{"bytes" => mls("text.bin")})
    assert_reply ref, :ok, %{at: at}
    assert_broadcast "msg", %{at: ^at}
    ref = push(a, "history", %{"after" => 0})
    assert_reply ref, :ok, %{messages: [%{at: _}, %{at: ^at}]}
  end

  test "Manage messages deletes a message by its number, never a commit", %{a: a, b: b, ka: ka} do
    ref = push(b, "relay", %{"bytes" => mls("text.bin")})
    assert_reply ref, :ok, %{seq: seq}
    ref = push(a, "delete", %{"seq" => seq})
    assert_reply ref, :error, %{reason: "You do not have permission to delete others' messages."}
    {:ok, mods} = Mod.role_save(@owner, %{"name" => "Moderator", "position" => 5, "perms" => ["manage_messages"]})
    :ok = Mod.role_give(@owner, ka, mods.id)
    ref = push(a, "delete", %{"seq" => 1})
    assert_reply ref, :error, %{reason: "That message is not in this room any more."}
    ref = push(a, "delete", %{"seq" => seq})
    assert_reply ref, :ok
    assert_broadcast "deleted", %{seq: ^seq}
    ref = push(a, "history", %{"after" => 0})
    assert_reply ref, :ok, %{messages: [%{seq: 1, commit: true}]}
    assert [%{action: "message deleted", actor: ^ka} | _] = Mod.audit(1)
  end

  test "a kicked or banned device is dropped, cannot connect or open a room, and members are told to take it out", %{kb: kb} do
    DenWeb.Endpoint.subscribe("device:" <> kb)
    :ok = Mod.ban(@owner, kb, "spam bot")
    :ok = Den.Rooms.expel(kb)
    assert_receive %Phoenix.Socket.Broadcast{event: "disconnect"}
    assert_broadcast "expired", %{keys: [^kb]}
    assert connect(DenWeb.UserSocket, %{"session" => session(kb)}) == :error
    refute kb in Den.Rooms.present("general")
  end
end
