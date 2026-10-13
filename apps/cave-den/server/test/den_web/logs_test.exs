defmodule DenWeb.LogsTest do
  # What the den writes to its log about a device: not its session, not its
  # key, not which private rooms it opens. Captured at debug level, the most
  # the den can log.
  use DenWeb.ChannelCase, async: false
  import ExUnit.CaptureLog

  test "a connect, an inbox and a private room leave no session, key or room in the log" do
    key = String.duplicate("7a", 32)
    Den.Rooms.admit(key, DateTime.add(DateTime.utc_now(), 3600, :second) |> DateTime.truncate(:second))
    session = Phoenix.Token.sign(DenWeb.Endpoint, "den", %{key: key})
    room = "p-" <> String.duplicate("5c", 16)
    {:ok, _} = Den.Rooms.private_room(room, key)
    previous = Logger.level()
    Logger.configure(level: :debug)

    log =
      capture_log([level: :debug], fn ->
        {:ok, socket} = connect(DenWeb.UserSocket, %{"session" => session})
        {:ok, _, _} = subscribe_and_join(socket, DenWeb.InboxChannel, "inbox:" <> key)
        {:ok, _, r} = subscribe_and_join(socket, DenWeb.RoomChannel, "room:" <> room)
        ref = push(r, "typing", %{})
        refute_reply ref, :ok, 50
        Process.sleep(50)
      end)

    Logger.configure(level: previous)
    refute log =~ session
    refute log =~ key
    refute log =~ room
  end

  test "the door's parameters are filtered" do
    filtered = Phoenix.Logger.filter_values(%{"code" => "c", "msg" => "m", "sig" => "s", "randomizer" => "r", "proof" => "p", "blinded" => "b", "session" => "x", "day" => "2026-10-13"})
    assert filtered == %{"code" => "[FILTERED]", "msg" => "[FILTERED]", "sig" => "[FILTERED]", "randomizer" => "[FILTERED]", "proof" => "[FILTERED]", "blinded" => "[FILTERED]", "session" => "[FILTERED]", "day" => "2026-10-13"}
  end

  test "no line the den logs, crash reports included, carries a key, a private room or a payload" do
    import ExUnit.CaptureLog
    key = String.duplicate("ab", 32)
    log = capture_log(fn ->
      require Logger
      Logger.error("crashed in room:p-#{String.duplicate("3c", 16)} for device #{key} with " <> String.duplicate("QUJD", 60))
    end)
    refute log =~ key
    refute log =~ String.duplicate("3c", 16)
    refute log =~ String.duplicate("QUJD", 60)
    assert log =~ "p-<private>" and log =~ "<hex>" and log =~ "<bytes>"
  end
end
