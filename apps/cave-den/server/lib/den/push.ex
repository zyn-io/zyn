defmodule Den.Push do
  @moduledoc """
  Notifications for a device whose page is closed (Web Push). A device signs
  up its browser's push address; when a message lands in a room it is in and
  it is not connected, the den sends it a push saying only the den's name and
  which room ("a private room" for one): the den cannot read the words.

  The push is encrypted for the browser (RFC 8291, aes128gcm), so the push
  service (Apple's, Google's, Mozilla's, Microsoft's) carries it without
  reading it, and is signed with the den's own key (VAPID, RFC 8292), made
  once and kept beside its day keys. Pushes go only to those services'
  addresses, never to an address of a member's choosing; at most one a
  device and room every #{120} seconds; a device takes them for every room
  ("all") or private rooms only ("private").

  The den learns a device's push address; the push service learns when a
  push reaches the device.
  """
  import Ecto.Query
  alias Den.{Repo, Rooms}

  @every 120
  @max_per_device 5
  @hosts ~w(fcm.googleapis.com web.push.apple.com)
  @suffixes ~w(.push.apple.com .push.services.mozilla.com .notify.windows.com)

  def init do
    if :ets.whereis(__MODULE__) == :undefined, do: :ets.new(__MODULE__, [:named_table, :public, :set, write_concurrency: true])
    :ok
  end

  # ---- The den's key ---------------------------------------------------------------

  defp key_file, do: Application.get_env(:den, :push_key_file) || Path.join(Path.dirname(Den.Keys.dir()), "push.key")

  @doc "The den's push key pair {public (65 bytes), private (32)}, made once."
  def keys do
    case :persistent_term.get({__MODULE__, :keys}, nil) do
      nil ->
        priv =
          case File.read(key_file()) do
            {:ok, b64} -> Base.decode64!(String.trim(b64))
            _ ->
              {_, priv} = :crypto.generate_key(:ecdh, :secp256r1)
              File.mkdir_p!(Path.dirname(key_file()))
              File.write!(key_file(), Base.encode64(priv))
              File.chmod(key_file(), 0o600)
              priv
          end

        {pub, _} = :crypto.generate_key(:ecdh, :secp256r1, priv)
        :persistent_term.put({__MODULE__, :keys}, {pub, priv})
        {pub, priv}

      k ->
        k
    end
  end

  @doc "The den's public push key, as the page gives it to the browser."
  def public_key, do: b64u(elem(keys(), 0))

  # ---- Who signs up -------------------------------------------------------------------

  @doc "A device's push address (or a new mode for it)."
  def subscribe(key, %{"endpoint" => ep, "p256dh" => p, "auth" => a} = s) when is_binary(ep) and is_binary(p) and is_binary(a) do
    mode = if s["mode"] == "all", do: "all", else: "private"

    with :ok <- endpoint_ok(ep),
         {:ok, pk} <- unb64u(p),
         true <- (byte_size(pk) == 65 and binary_part(pk, 0, 1) == <<4>>) || {:error, :push_key},
         {:ok, auth} <- unb64u(a),
         true <- byte_size(auth) == 16 || {:error, :push_key} do
      Repo.delete_all(from s in "push_subs", where: s.endpoint == ^ep)
      Repo.insert_all("push_subs", [%{endpoint: ep, key: key, p256dh: p, auth: a, mode: mode, made_at: Rooms.now()}])
      # The newest few a device: an old one it lost is dropped.
      old =
        Repo.all(from s in "push_subs", where: s.key == ^key, order_by: [desc: s.made_at, desc: fragment("rowid")], select: s.endpoint)
        |> Enum.drop(@max_per_device)

      if old != [], do: Repo.delete_all(from s in "push_subs", where: s.endpoint in ^old)
      :ok
    end
  end

  def subscribe(_, _), do: {:error, :push_sub}

  def unsubscribe(key, ep) when is_binary(ep), do: Repo.delete_all(from s in "push_subs", where: s.key == ^key and s.endpoint == ^ep) && :ok
  def unsubscribe(_, _), do: :ok

  @doc "Only the push services' own addresses, over https."
  def endpoint_ok(ep) do
    case URI.parse(ep) do
      %URI{scheme: "https", host: host, port: 443} when is_binary(host) and byte_size(ep) <= 1024 ->
        if host in @hosts or Enum.any?(@suffixes, &String.ends_with?(host, &1)), do: :ok, else: {:error, :push_service}

      _ ->
        {:error, :push_service}
    end
  end

  # ---- Sending --------------------------------------------------------------------------

  @doc "A message landed in `room` from `sender`: push to its members not connected."
  def notify(room, sender) do
    if Process.whereis(Den.Push.Tasks), do: Task.Supervisor.start_child(Den.Push.Tasks, fn -> run(room, sender) end)
    :ok
  end

  def run(room_id, sender) do
    with %{} = room <- Rooms.get(room_id) do
      online = MapSet.new(for {_, k} <- Registry.lookup(Den.Online, room_id), do: k)
      targets = Enum.reject(Rooms.members(room), &(&1 == sender or MapSet.member?(online, &1) or Den.Mod.out?(&1)))

      subs =
        if targets == [], do: [], else: Repo.all(from s in "push_subs", where: s.key in ^targets, select: %{endpoint: s.endpoint, p256dh: s.p256dh, auth: s.auth, mode: s.mode})

      body = payload(room)
      send = Application.get_env(:den, :push_deliver, &deliver/2)
      for s <- subs, s.mode == "all" or room.kind == "private", due?(s.endpoint, room_id), do: send.(s, body)
    end

    :ok
  end

  defp payload(room) do
    name = Den.Mod.settings()["name"]
    where = if room.kind == "private", do: "a private room", else: room.title || room.id
    Jason.encode!(%{title: if(name in [nil, ""], do: "Epeaky", else: name), body: "New message in #{where}", tag: room.id})
  end

  defp due?(endpoint, room) do
    now = System.monotonic_time(:second)

    case :ets.lookup(__MODULE__, {endpoint, room}) do
      [{_, t}] when now - t < @every -> false
      _ -> :ets.insert(__MODULE__, {{endpoint, room}, now}) && true
    end
  end

  defp deliver(s, plain) do
    {:ok, ua} = unb64u(s.p256dh)
    {:ok, auth} = unb64u(s.auth)
    body = encrypt(plain, ua, auth)
    %URI{scheme: scheme, host: host} = URI.parse(s.endpoint)

    headers = [
      {~c"ttl", ~c"3600"},
      {~c"urgency", ~c"normal"},
      {~c"content-encoding", ~c"aes128gcm"},
      {~c"authorization", String.to_charlist("vapid t=#{jwt("#{scheme}://#{host}")}, k=#{public_key()}")}
    ]

    ssl = [verify: :verify_peer, cacerts: :public_key.cacerts_get(), customize_hostname_check: [match_fun: :public_key.pkix_verify_hostname_match_fun(:https)]]

    case :httpc.request(:post, {String.to_charlist(s.endpoint), headers, ~c"application/octet-stream", body}, [ssl: ssl, timeout: 10_000, connect_timeout: 5_000], []) do
      # Gone: the browser dropped it.
      {:ok, {{_, code, _}, _, _}} when code in [404, 410] -> Repo.delete_all(from x in "push_subs", where: x.endpoint == ^s.endpoint)
      _ -> :ok
    end
  rescue
    _ -> :ok
  end

  @doc "Push addresses not refreshed in 60 days: their page is gone."
  def sweep do
    Repo.delete_all(from s in "push_subs", where: s.made_at < ^DateTime.add(Rooms.now(), -60 * 86_400, :second))
    :ok
  end

  # ---- RFC 8291 and 8292 ------------------------------------------------------------------

  @doc "The push message for a browser: aes128gcm, one record."
  def encrypt(plain, ua_public, auth_secret) do
    {as_public, as_private} = :crypto.generate_key(:ecdh, :secp256r1)
    secret = :crypto.compute_key(:ecdh, ua_public, as_private, :secp256r1)
    prk_key = hmac(auth_secret, secret)
    ikm = hmac(prk_key, "WebPush: info" <> <<0>> <> ua_public <> as_public <> <<1>>)
    salt = :crypto.strong_rand_bytes(16)
    prk = hmac(salt, ikm)
    cek = binary_part(hmac(prk, "Content-Encoding: aes128gcm" <> <<0, 1>>), 0, 16)
    nonce = binary_part(hmac(prk, "Content-Encoding: nonce" <> <<0, 1>>), 0, 12)
    {ct, tag} = :crypto.crypto_one_time_aead(:aes_128_gcm, cek, nonce, plain <> <<2>>, <<>>, true)
    salt <> <<4096::32, 65::8>> <> as_public <> ct <> tag
  end

  @doc "The den's signed token for one push service (audience: its origin)."
  def jwt(aud) do
    {_, priv} = keys()
    head = b64u(Jason.encode!(%{typ: "JWT", alg: "ES256"}))
    sub = DenWeb.Endpoint.url()
    sub = if String.starts_with?(sub, "https://"), do: sub, else: "mailto:den@localhost"
    claims = b64u(Jason.encode!(%{aud: aud, exp: System.os_time(:second) + 12 * 3600, sub: sub}))
    input = head <> "." <> claims
    der = :crypto.sign(:ecdsa, :sha256, input, [priv, :secp256r1])
    {:"ECDSA-Sig-Value", r, s} = :public_key.der_decode(:"ECDSA-Sig-Value", der)
    input <> "." <> b64u(<<r::256, s::256>>)
  end

  defp hmac(k, d), do: :crypto.mac(:hmac, :sha256, k, d)
  defp b64u(b), do: Base.url_encode64(b, padding: false)

  defp unb64u(s) do
    case Base.url_decode64(s, padding: false) do
      {:ok, b} -> {:ok, b}
      :error -> {:error, :push_key}
    end
  end
end
