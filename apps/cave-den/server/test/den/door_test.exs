defmodule Den.DoorTest do
  use Den.DataCase, async: false

  @f Path.expand("../fixtures", __DIR__)

  setup do
    Application.put_env(:den, :keys_dir, Path.join(@f, "keys"))
    Application.put_env(:den, :today, fn -> ~D[2026-10-12] end)
    %{"code" => code} = Jason.decode!(File.read!(Path.join(@f, "pass.json")))
    blinded = String.trim(File.read!(Path.join(@f, "blinded.hex")))
    Application.put_env(:den, :now, fn -> ~U[2026-10-12 22:30:00Z] end)
    on_exit(fn -> Enum.each([:today, :now, :gate_check], &Application.delete_env(:den, &1)) end)
    %{code: code, blinded: blinded}
  end

  defp ledger(answer), do: Application.put_env(:den, :gate_check, fn _ -> answer end)

  test "a live pass gets three tokens a day, no more", %{code: code, blinded: b} do
    ledger({:ok, true})
    for _ <- 1..3, do: assert({:ok, _} = Den.Door.sign(code, "2026-10-12", b))
    assert Den.Door.sign(code, "2026-10-12", b) == {:error, :limit}
    # Tomorrow's are counted apart (and that key is not made here).
    assert Den.Door.sign(code, "2026-10-13", b) == {:error, :no_key}
  end

  # A pass the host removes before the evening lets nobody in tomorrow: no
  # device can hold tomorrow's token from earlier in the day.
  test "tomorrow's token only in the day's last hours", %{code: code, blinded: b} do
    on_exit(fn -> Application.delete_env(:den, :tomorrow_from_hour) end)
    ledger({:ok, true})
    Application.put_env(:den, :now, fn -> ~U[2026-10-12 21:59:00Z] end)
    assert Den.Door.sign(code, "2026-10-13", b) == {:error, :early}
    assert {:ok, _} = Den.Door.sign(code, "2026-10-12", b)
    Application.put_env(:den, :now, fn -> ~U[2026-10-12 22:00:00Z] end)
    assert Den.Door.sign(code, "2026-10-13", b) == {:error, :no_key}, "past the hour: on to the day key"
    Application.put_env(:den, :tomorrow_from_hour, 0)
    Application.put_env(:den, :now, fn -> ~U[2026-10-12 08:00:00Z] end)
    assert Den.Door.sign(code, "2026-10-13", b) == {:error, :no_key}
  end

  test "a dead pass, a wrong day, a bad code, a silent gate", %{code: code, blinded: b} do
    ledger({:ok, false})
    assert Den.Door.sign(code, "2026-10-12", b) == {:error, :not_live}
    ledger({:ok, true})
    assert Den.Door.sign(code, "2026-10-14", b) == {:error, :day}
    assert Den.Door.sign(code, "2026-10-11", b) == {:error, :day}
    assert Den.Door.sign("zz", "2026-10-12", b) == {:error, :code}
    assert Den.Door.sign(String.duplicate("AB", 32), "2026-10-12", b) == {:error, :code}, "lower-case hex only"
    assert Den.Door.sign(code <> "\n", "2026-10-12", b) == {:error, :code}
    ledger({:error, :down})
    assert Den.Door.sign(code, "2026-10-12", b) == {:error, :gate}
  end

  # A device and a token for it, signed with the test-only day key (as the
  # door's blind signature unblinds to), and the device's proof of its key.
  defp device do
    {pk, sk} = :crypto.generate_key(:eddsa, :ed25519)
    {Base.encode16(pk, case: :lower), sk}
  end

  defp token(key, day \\ "2026-10-12") do
    msg = "den-token-v1|#{day}|#{key}|#{Base.encode16(:crypto.strong_rand_bytes(16), case: :lower)}"
    randomizer = :crypto.strong_rand_bytes(32)
    {:ok, k} = Den.Keys.private("2026-10-12")
    sig = :public_key.sign(randomizer <> msg, :sha384, k, rsa_padding: :rsa_pkcs1_pss_padding, rsa_pss_saltlen: 48)
    {msg, Base.encode16(randomizer, case: :lower), Base.encode16(sig, case: :lower)}
  end

  defp proof(msg, sk), do: :crypto.sign(:eddsa, :none, "den-enter-v1|" <> msg, [sk, :ed25519]) |> Base.encode16(case: :lower)

  test "a token admits its device on its day, once, when the device proves its key" do
    {key, sk} = device()
    {msg, r, sig} = token(key)
    assert {:ok, %{session: s, key: ^key, until: until}} = Den.Door.enter(msg, r, sig, proof(msg, sk))
    assert until == DateTime.to_unix(~U[2026-10-13 01:00:00Z])
    assert {:ok, %{key: ^key}} = Phoenix.Token.verify(DenWeb.Endpoint, "den", s, max_age: 3 * 86_400)
    assert Den.Door.enter(msg, r, sig, proof(msg, sk)) == {:error, :spent}
    assert Den.Door.enter(String.replace(msg, "2026-10-12", "2026-10-13"), r, sig, proof(msg, sk)) == {:error, :day}
    assert Den.Door.enter("den-token-v1|2026-10-12|nope|x", r, sig, nil) == {:error, :token}
  end

  test "a device the den's moderators kicked or banned is refused at the door, and its token is not spent" do
    {key, sk} = device()
    {msg, r, sig} = token(key)
    owner = String.duplicate("0a", 32)
    :ok = Den.Mod.set_owner(owner)
    :ok = Den.Mod.ban(owner, key, "spam")
    assert Den.Door.enter(msg, r, sig, proof(msg, sk)) == {:error, :out}
    :ok = Den.Mod.unban(owner, key)
    assert {:ok, _} = Den.Door.enter(msg, r, sig, proof(msg, sk))
  end

  test "a member cannot enter as another device: the token names a key the member does not hold" do
    {victim, _victim_sk} = device()
    {_mine, my_sk} = device()
    {msg, r, sig} = token(victim)
    assert Den.Door.enter(msg, r, sig, proof(msg, my_sk)) == {:error, :proof}
    assert Den.Door.enter(msg, r, sig, nil) == {:error, :proof}
    assert Den.Door.enter(msg, r, sig, String.duplicate("00", 64)) == {:error, :proof}
    # A refused proof spends nothing: the right device can still use its token.
    {key, sk} = device()
    {msg2, r2, sig2} = token(key)
    assert Den.Door.enter(msg2, r2, sig2, proof(msg, sk)) == {:error, :proof}, "a proof of another message"
    assert {:ok, _} = Den.Door.enter(msg2, r2, sig2, proof(msg2, sk))
  end

  test "the fixture token from the browser's blinding still verifies, and still needs a proof" do
    %{"msg" => msg, "randomizer" => r, "sig" => sig} = Jason.decode!(File.read!(Path.join(@f, "token.json")))
    assert Den.Door.enter(msg, r, sig, nil) == {:error, :proof}
  end

  test "a token for another day is refused on this one" do
    Application.put_env(:den, :today, fn -> ~D[2026-10-13] end)
    %{"msg" => msg, "randomizer" => r, "sig" => sig} = Jason.decode!(File.read!(Path.join(@f, "token.json")))
    assert Den.Door.enter(msg, r, sig, nil) == {:error, :day}
  end
end
