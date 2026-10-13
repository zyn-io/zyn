defmodule Den.HygieneTest do
  # The den's smaller duties: which addresses voice offers, warning before
  # the day keys run out, files written whole and kept their full time, and
  # two makers of one room.
  use Den.DataCase, async: false
  import ExUnit.CaptureLog

  test "voice offers no loopback, link-local or Docker address; with public_only, no private one" do
    for ip <- [{127, 0, 0, 1}, {169, 254, 1, 2}, {172, 17, 0, 1}, {0, 0, 0, 0, 0, 0, 0, 1}, {0xFE80, 0, 0, 0, 0, 0, 0, 1}], do: refute(Den.Voice.ip_ok?(ip))
    for ip <- [{203, 0, 113, 24}, {192, 168, 50, 10}, {10, 0, 0, 5}], do: assert(Den.Voice.ip_ok?(ip))
    Application.put_env(:den, :voice_public_only, true)
    refute Den.Voice.ip_ok?({192, 168, 50, 10})
    refute Den.Voice.ip_ok?({10, 0, 0, 5})
    assert Den.Voice.ip_ok?({203, 0, 113, 24})
    Application.delete_env(:den, :voice_public_only)
  end

  test "the door says when its day keys run out" do
    dir = Path.join(System.tmp_dir!(), "den-keys-#{System.unique_integer([:positive])}")
    was = Application.get_env(:den, :keys_dir)
    Application.put_env(:den, :keys_dir, dir)
    on_exit(fn -> File.rm_rf(dir) && Application.put_env(:den, :keys_dir, was) end)
    today = ~D[2026-10-13]
    assert Den.Keys.days_ahead(today) == 0
    assert capture_log(fn -> Den.Keys.check(today) end) =~ "no key for today"
    for d <- 0..2, do: Den.Keys.make(Date.to_iso8601(Date.add(today, d)))
    assert Den.Keys.days_ahead(today) == 3
    assert capture_log(fn -> Den.Keys.check(today) end) =~ "day keys for 3 more days"
  end

  test "a file is written whole, and sent again is kept again" do
    dir = Path.join(System.tmp_dir!(), "den-blobs-#{System.unique_integer([:positive])}")
    Application.put_env(:den, :blobs_dir, dir)
    on_exit(fn -> File.rm_rf(dir) && Application.delete_env(:den, :blobs_dir) end)
    {:ok, id} = Den.Blobs.put("sealed bytes")
    assert File.ls!(dir) == [id], "no part-file left behind"
    Repo.update_all(from(b in "blobs", where: b.id == ^id), set: [inserted_at: ~U[2026-01-01 00:00:00Z]])
    {:ok, ^id} = Den.Blobs.put("sealed bytes")
    assert Repo.one(from b in "blobs", where: b.id == ^id, select: type(b.inserted_at, :utc_datetime)) |> DateTime.compare(~U[2026-01-02 00:00:00Z]) == :gt
    assert Den.Blobs.path(id <> "\n") == :error
  end

  test "a private room made twice at once: one is made, the other told it exists" do
    id = "p-" <> String.duplicate("9a", 16)
    results = [Task.async(fn -> Den.Rooms.private_room(id) end), Task.async(fn -> Den.Rooms.private_room(id) end)] |> Enum.map(&Task.await/1)
    assert Enum.count(results, &match?({:ok, _}, &1)) == 1
    assert Enum.count(results, &(&1 == {:error, :exists})) == 1
    assert Den.Rooms.private_room(id <> "\n") == {:error, :id}
  end

  test "a private room nobody wrote or sat in for the retention time is swept; one in use stays" do
    idle = "p-" <> String.duplicate("6a", 16)
    used = "p-" <> String.duplicate("6b", 16)
    {:ok, _} = Den.Rooms.private_room(idle, String.duplicate("aa", 32))
    {:ok, _} = Den.Rooms.private_room(used, String.duplicate("aa", 32))
    Den.Rooms.seat(used, String.duplicate("ac", 32))
    Repo.update_all(from(r in Den.Rooms.Room, where: r.id in ^[idle, used]), set: [updated_at: ~U[2025-01-01 00:00:00Z]])
    Den.Rooms.sweep()
    assert Den.Rooms.get(idle) == nil
    assert Den.Rooms.get(used) != nil
  end
end
