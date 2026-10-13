defmodule Den.Gate.ZynTest do
  use Den.DataCase, async: false

  @f Path.expand("../../test/fixtures", __DIR__)
  # Nap's pass for the fixture code 11…11: the ledger's objectvm::dao::pass_of.
  @pass "85ec6f7e959bcca261797324ccdeac6b50bad226522a6371a6ca66e9134f7c1d"

  setup do
    Application.put_env(:den, :gate, Den.Gate.Zyn)
    Application.put_env(:den, :keys_dir, Path.join(@f, "keys"))
    Application.put_env(:den, :today, fn -> ~D[2026-10-12] end)
    %{"code" => code} = Jason.decode!(File.read!(Path.join(@f, "pass.json")))
    on_exit(fn ->
      Application.put_env(:den, :gate, Den.Gate.Members)
      Application.delete_env(:den, :today)
      Application.delete_env(:den, :zyn_check)
    end)
    %{code: code, blinded: String.trim(File.read!(Path.join(@f, "blinded.hex")))}
  end

  test "a pass is the ledger's hash of its code, as Nap computes it", %{code: code} do
    {:ok, c} = Den.Gate.code(code)
    assert Base.encode16(Den.Gate.Zyn.id(c), case: :lower) == @pass
  end

  test "the door's rules hold with the Zyn gate: three a day, a dead pass, a silent ledger", %{code: code, blinded: b} do
    Application.put_env(:den, :zyn_check, fn pass -> {:ok, Base.encode16(pass, case: :lower) == @pass} end)
    for _ <- 1..3, do: assert({:ok, _} = Den.Door.sign(code, "2026-10-12", b))
    assert Den.Door.sign(code, "2026-10-12", b) == {:error, :limit}
    assert Den.Door.sign(String.duplicate("22", 32), "2026-10-12", b) == {:error, :not_live}
    Application.put_env(:den, :zyn_check, fn _ -> {:error, :down} end)
    assert Den.Door.sign(String.duplicate("33", 32), "2026-10-12", b) == {:error, :gate}
  end
end
