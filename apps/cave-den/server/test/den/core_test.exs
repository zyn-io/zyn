defmodule Den.CoreTest do
  use ExUnit.Case, async: true

  @f Path.expand("../fixtures", __DIR__)

  test "a pass is the ledger's hash of its code, as Nap computes it" do
    %{"code" => code, "pass" => pass} = Jason.decode!(File.read!(Path.join(@f, "pass.json")))
    {:ok, c} = Den.Pass.code(code)
    assert Base.encode16(Den.Pass.of(c), case: :lower) == pass
    assert Den.Pass.code("zz") == :error
    assert Den.Pass.code(String.duplicate("A", 64)) == :error, "lower-case hex only"
  end

  test "the envelope of real MLS messages: room, epoch, commit, external" do
    env = fn f -> Den.Mls.envelope(File.read!(Path.join(@f, f))) end
    assert env.("join.bin") == {:ok, %{room: "general", epoch: 0, commit: true, external: true}}
    assert env.("text.bin") == {:ok, %{room: "general", epoch: 1, commit: false, external: false}}
    assert env.("remove.bin") == {:ok, %{room: "general", epoch: 2, commit: true, external: false}}
    assert Den.Mls.envelope(<<1, 2, 3>>) == {:error, :not_a_room_message}
    assert Den.Mls.envelope(<<0, 1, 0, 2, 0x40>>) == {:error, :malformed}
  end

  test "a day key signs blind, and only its own day's tokens verify" do
    Application.put_env(:den, :keys_dir, Path.join(@f, "keys"))
    blinded = Base.decode16!(String.trim(File.read!(Path.join(@f, "blinded.hex"))), case: :lower)
    assert {:ok, s} = Den.Keys.blind_sign("2026-10-12", blinded)
    assert byte_size(s) == 256
    assert Den.Keys.blind_sign("2026-10-13", blinded) == {:error, :no_key}
    assert Den.Keys.blind_sign("2026-10-12", <<1, 2>>) == {:error, :wrong_length}

    %{"msg" => msg, "randomizer" => r, "sig" => sig} = Jason.decode!(File.read!(Path.join(@f, "token.json")))
    {r, sig} = {Base.decode16!(r, case: :lower), Base.decode16!(sig, case: :lower)}
    assert Den.Keys.valid?("2026-10-12", msg, r, sig)
    refute Den.Keys.valid?("2026-10-12", msg <> "x", r, sig)
    refute Den.Keys.valid?("2026-10-13", msg, r, sig), "no key, no entry"
    assert Den.Keys.published(~D[2026-10-12], 2) |> Map.keys() == ["2026-10-12"]
  end
end
