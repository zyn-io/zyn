defmodule Den.DoorTest do
  use Den.DataCase, async: false

  @f Path.expand("../fixtures", __DIR__)

  setup do
    Application.put_env(:den, :keys_dir, Path.join(@f, "keys"))
    Application.put_env(:den, :today, fn -> ~D[2026-10-12] end)
    %{"code" => code} = Jason.decode!(File.read!(Path.join(@f, "pass.json")))
    blinded = String.trim(File.read!(Path.join(@f, "blinded.hex")))
    on_exit(fn -> Application.delete_env(:den, :today) && Application.delete_env(:den, :ledger_check) end)
    %{code: code, blinded: blinded}
  end

  defp ledger(answer), do: Application.put_env(:den, :ledger_check, fn _ -> answer end)

  test "a live pass gets three tokens a day, no more", %{code: code, blinded: b} do
    ledger({:ok, true})
    for _ <- 1..3, do: assert({:ok, _} = Den.Door.sign(code, "2026-10-12", b))
    assert Den.Door.sign(code, "2026-10-12", b) == {:error, :limit}
    # Tomorrow's are counted apart (and that key is not made here).
    assert Den.Door.sign(code, "2026-10-13", b) == {:error, :no_key}
  end

  test "a dead pass, a wrong day, a bad code, a silent ledger", %{code: code, blinded: b} do
    ledger({:ok, false})
    assert Den.Door.sign(code, "2026-10-12", b) == {:error, :not_live}
    ledger({:ok, true})
    assert Den.Door.sign(code, "2026-10-14", b) == {:error, :day}
    assert Den.Door.sign(code, "2026-10-11", b) == {:error, :day}
    assert Den.Door.sign("zz", "2026-10-12", b) == {:error, :code}
    ledger({:error, :down})
    assert Den.Door.sign(code, "2026-10-12", b) == {:error, :ledger}
  end

  test "a token admits its device on its day, once" do
    %{"msg" => msg, "randomizer" => r, "sig" => sig} = Jason.decode!(File.read!(Path.join(@f, "token.json")))
    assert {:ok, %{session: s, key: key, until: until}} = Den.Door.enter(msg, r, sig)
    assert key == String.duplicate("ab", 32)
    assert until == DateTime.to_unix(~U[2026-10-13 01:00:00Z])
    assert {:ok, %{key: ^key}} = Phoenix.Token.verify(DenWeb.Endpoint, "den", s, max_age: 3 * 86_400)
    assert Den.Door.enter(msg, r, sig) == {:error, :spent}
    assert Den.Door.enter(String.replace(msg, "2026-10-12", "2026-10-13"), r, sig) == {:error, :day}
    assert Den.Door.enter("cave-den-token-v1|2026-10-12|nope|x", r, sig) == {:error, :token}
  end

  test "a token for another day is refused on this one" do
    Application.put_env(:den, :today, fn -> ~D[2026-10-13] end)
    %{"msg" => msg, "randomizer" => r, "sig" => sig} = Jason.decode!(File.read!(Path.join(@f, "token.json")))
    assert Den.Door.enter(msg, r, sig) == {:error, :day}
  end
end
