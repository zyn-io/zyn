defmodule Den.PushTest do
  use DenWeb.ConnCase, async: false
  import Ecto.Query
  alias Den.{Push, Repo}
  alias Den.Rooms.Room

  setup do
    file = Path.join(System.tmp_dir!(), "den-push-#{System.unique_integer([:positive])}.key")
    Application.put_env(:den, :push_key_file, file)
    :persistent_term.erase({Push, :keys})
    Push.init()
    :ets.delete_all_objects(Push)
    me = self()
    Application.put_env(:den, :push_deliver, fn s, body -> send(me, {:pushed, s.endpoint, body}) end)
    Application.put_env(:den, :push_jitter_ms, 0)

    on_exit(fn ->
      File.rm(file)
      :persistent_term.erase({Push, :keys})
      Application.delete_env(:den, :push_key_file)
      Application.delete_env(:den, :push_deliver)
      Application.delete_env(:den, :push_jitter_ms)
    end)

    :ok
  end

  defp b64u(b), do: Base.url_encode64(b, padding: false)
  defp key, do: Base.encode16(:crypto.strong_rand_bytes(32), case: :lower)

  defp browser do
    {pub, priv} = :crypto.generate_key(:ecdh, :secp256r1)
    auth = :crypto.strong_rand_bytes(16)
    {%{"p256dh" => b64u(pub), "auth" => b64u(auth)}, pub, priv, auth}
  end

  test "only the push services' addresses, over https" do
    for ep <- ~w(https://fcm.googleapis.com/fcm/send/abc https://web.push.apple.com/QGx https://api.push.apple.com/x
                 https://updates.push.services.mozilla.com/wpush/v2/x https://wns2-par02p.notify.windows.com/w/?token=x),
        do: assert(Push.endpoint_ok(ep) == :ok, ep)

    for ep <- ~w(http://fcm.googleapis.com/x https://fcm.googleapis.com.evil.example/x https://evil.example/push.apple.com
                 https://localhost/x https://fcm.googleapis.com:8443/x https://169.254.169.254/latest
                 https://user@fcm.googleapis.com/x https://evil.example%2f.push.apple.com/x https://FCM.googleapis.com/x) ++
                 ["https://evil.example\\.push.apple.com/x", "https://fcm.googleapis.com/x\n", 42],
        do: assert(Push.endpoint_ok(ep) == {:error, :push_service}, inspect(ep))
  end

  test "the message decrypts as RFC 8291 says, for the browser alone" do
    {_, ua_pub, ua_priv, auth} = browser()
    body = Push.encrypt(~s({"title":"Builders"}), ua_pub, auth)
    <<salt::binary-16, 4096::32, 65::8, as_pub::binary-65, rest::binary>> = body
    secret = :crypto.compute_key(:ecdh, as_pub, ua_priv, :secp256r1)
    hmac = &:crypto.mac(:hmac, :sha256, &1, &2)
    ikm = hmac.(hmac.(auth, secret), "WebPush: info" <> <<0>> <> ua_pub <> as_pub <> <<1>>)
    prk = hmac.(salt, ikm)
    cek = binary_part(hmac.(prk, "Content-Encoding: aes128gcm" <> <<0, 1>>), 0, 16)
    nonce = binary_part(hmac.(prk, "Content-Encoding: nonce" <> <<0, 1>>), 0, 12)
    ct = binary_part(rest, 0, byte_size(rest) - 16)
    tag = binary_part(rest, byte_size(rest) - 16, 16)
    assert :crypto.crypto_one_time_aead(:aes_128_gcm, cek, nonce, ct, <<>>, tag, false) == ~s({"title":"Builders"}) <> <<2>>
    # Another browser's key opens nothing.
    {_, other_priv} = :crypto.generate_key(:ecdh, :secp256r1)
    wrong = :crypto.compute_key(:ecdh, as_pub, other_priv, :secp256r1)
    wprk = hmac.(salt, hmac.(hmac.(auth, wrong), "WebPush: info" <> <<0>> <> ua_pub <> as_pub <> <<1>>))
    wcek = binary_part(hmac.(wprk, "Content-Encoding: aes128gcm" <> <<0, 1>>), 0, 16)
    wnonce = binary_part(hmac.(wprk, "Content-Encoding: nonce" <> <<0, 1>>), 0, 12)
    assert :crypto.crypto_one_time_aead(:aes_128_gcm, wcek, wnonce, ct, <<>>, tag, false) == :error
  end

  test "the den's token is signed with its key, for the push service's origin" do
    [h, c, sig] = String.split(Push.jwt("https://fcm.googleapis.com"), ".")
    assert Jason.decode!(Base.url_decode64!(h, padding: false)) == %{"typ" => "JWT", "alg" => "ES256"}
    claims = Jason.decode!(Base.url_decode64!(c, padding: false))
    assert claims["aud"] == "https://fcm.googleapis.com" and claims["exp"] > System.os_time(:second)
    <<r::256, s::256>> = Base.url_decode64!(sig, padding: false)
    der = :public_key.der_encode(:"ECDSA-Sig-Value", {:"ECDSA-Sig-Value", r, s})
    {pub, _} = Push.keys()
    assert :crypto.verify(:ecdsa, :sha256, h <> "." <> c, der, [pub, :secp256r1])
    # The key is kept: the same one after a restart.
    :persistent_term.erase({Push, :keys})
    assert elem(Push.keys(), 0) == pub
  end

  test "a device signs up a few addresses; a bad one is refused" do
    k = key()
    {keys, _, _, _} = browser()
    assert Push.subscribe(k, Map.put(keys, "endpoint", "https://evil.example/x")) == {:error, :push_service}
    assert Push.subscribe(k, %{"endpoint" => "https://fcm.googleapis.com/fcm/send/x", "p256dh" => "AAAA", "auth" => keys["auth"]}) == {:error, :push_key}
    for i <- 1..7, do: assert(Push.subscribe(k, Map.put(keys, "endpoint", "https://fcm.googleapis.com/fcm/send/#{i}")) == :ok)
    assert Repo.aggregate(from(s in "push_subs", where: s.key == ^k), :count) == 5
    # Another device cannot take one over.
    assert Push.subscribe(key(), Map.put(keys, "endpoint", "https://fcm.googleapis.com/fcm/send/7")) == {:error, :push_taken}
    assert Repo.one(from s in "push_subs", where: s.endpoint == "https://fcm.googleapis.com/fcm/send/7", select: s.key) == k
  end

  test "who was pushed when is kept only while it holds the next push back" do
    now = System.monotonic_time(:second)
    :ets.insert(Push, {{"https://fcm.googleapis.com/fcm/send/old", "r"}, now - 500})
    :ets.insert(Push, {{"https://fcm.googleapis.com/fcm/send/new", "r"}, now})
    Push.sweep()
    assert :ets.lookup(Push, {"https://fcm.googleapis.com/fcm/send/old", "r"}) == []
    assert [_] = :ets.lookup(Push, {"https://fcm.googleapis.com/fcm/send/new", "r"})
  end

  test "a room's pushes all leave within the jitter window, not one window after another" do
    {keys, _, _, _} = browser()
    members = for i <- 1..80, do: (k = key(); :ok = Push.subscribe(k, Map.merge(keys, %{"endpoint" => "https://fcm.googleapis.com/fcm/send/m#{i}", "mode" => "all"})); k)
    Repo.insert!(%Room{id: "big-push", title: "Big", kind: "open", members: Enum.join(members, ",")})
    me = self()
    Application.put_env(:den, :push_deliver, fn s, _ -> Process.sleep(20); send(me, {:pushed, s.endpoint}) end)
    Application.put_env(:den, :push_jitter_ms, 400)
    {us, :ok} = :timer.tc(fn -> Push.run("big-push", key()) end)
    for _ <- 1..80, do: assert_received({:pushed, "https://fcm.googleapis.com/fcm/send/m" <> _})
    assert us < 1_500_000, "took #{div(us, 1000)} ms"
  end

  test "a push goes to members not connected, by their mode, once in a while" do
    [sender, priv_mode, all_mode] = for _ <- 1..3, do: key()
    {keys, _, _, _} = browser()
    :ok = Push.subscribe(priv_mode, Map.merge(keys, %{"endpoint" => "https://fcm.googleapis.com/fcm/send/p", "mode" => "private"}))
    :ok = Push.subscribe(all_mode, Map.merge(keys, %{"endpoint" => "https://fcm.googleapis.com/fcm/send/a", "mode" => "all"}))
    :ok = Push.subscribe(sender, Map.merge(keys, %{"endpoint" => "https://fcm.googleapis.com/fcm/send/s", "mode" => "all"}))
    members = Enum.join([sender, priv_mode, all_mode], ",")
    Repo.insert!(%Room{id: "open-push", title: "Ops", kind: "open", members: members})
    Repo.insert!(%Room{id: "p-" <> String.duplicate("a", 32), title: "", kind: "private", members: members})

    Push.run("open-push", sender)
    assert_received {:pushed, "https://fcm.googleapis.com/fcm/send/a", body}
    assert Jason.decode!(body)["body"] == "New message in Ops"
    refute_received {:pushed, "https://fcm.googleapis.com/fcm/send/p", _}
    refute_received {:pushed, "https://fcm.googleapis.com/fcm/send/s", _}

    Push.run("p-" <> String.duplicate("a", 32), sender)
    assert_received {:pushed, "https://fcm.googleapis.com/fcm/send/p", body}
    assert Jason.decode!(body)["body"] == "New message in a private room"
    # "all" takes private rooms too.
    assert_received {:pushed, "https://fcm.googleapis.com/fcm/send/a", _}
    # Once in a while: not again for the same room straight away.
    Push.run("open-push", sender)
    refute_received {:pushed, "https://fcm.googleapis.com/fcm/send/a", _}
    # Not to a device that is connected to the room.
    :ets.delete_all_objects(Push)
    {:ok, _} = Registry.register(Den.Online, "open-push", all_mode)
    Push.run("open-push", sender)
    refute_received {:pushed, "https://fcm.googleapis.com/fcm/send/a", _}
  end
end
