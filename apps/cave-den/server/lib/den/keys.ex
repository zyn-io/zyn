defmodule Den.Keys do
  @moduledoc """
  The door's day keys (RFC 9474, RSABSSA-SHA384-PSS-Randomized). Each UTC day
  has its own RSA key: the door signs a blinded token for a day only on that
  day or the day before, and the den admits a token only on its day, so a
  holder who sells their item cannot have saved tokens for later.

  Every key is published in advance, the same file for everyone
  (`cavebro.win/den-keys.json`), and the page uses only those — so the door
  cannot hand each holder a different key to recognise them by.

  Private keys live in `keys_dir`, one PEM per day (`YYYY-MM-DD.pem`), made
  ahead with `mix den.keys`.
  """

  @salt 48

  def dir, do: Application.fetch_env!(:den, :keys_dir)

  @doc "The day's private key, or :error."
  def private(day) do
    with true <- day =~ ~r/\A\d{4}-\d{2}-\d{2}\z/,
         {:ok, pem} <- File.read(Path.join(dir(), day <> ".pem")),
         [entry] <- :public_key.pem_decode(pem) do
      {:ok, :public_key.pem_entry_decode(entry)}
    else
      _ -> :error
    end
  end

  defp public_of({:RSAPrivateKey, _, n, e, _, _, _, _, _, _, _}), do: {:RSAPublicKey, n, e}

  @doc "The day's public key as SubjectPublicKeyInfo DER, as the page's library reads it."
  def public_der(day) do
    with {:ok, key} <- private(day) do
      {:ok, :public_key.der_encode(:SubjectPublicKeyInfo, spki(public_of(key)))}
    end
  end

  defp spki({:RSAPublicKey, _, _} = pub) do
    {:SubjectPublicKeyInfo, {:AlgorithmIdentifier, {1, 2, 840, 113_549, 1, 1, 1}, <<5, 0>>}, :public_key.der_encode(:RSAPublicKey, pub)}
  end

  @doc """
  Every day key from `from` for `days` days, public halves: %{day => base64
  DER}. Read from the private keys once per day and kept: `/door/keys` is
  asked by anyone, and decoding 31 PEMs a request is work for nothing.
  """
  def published(from, days) do
    cache = {__MODULE__, :published, from, days, dir()}

    case :persistent_term.get(cache, nil) do
      nil ->
        keys = for d <- 0..(days - 1), day = Date.to_iso8601(Date.add(from, d)), {:ok, der} <- [public_der(day)], into: %{}, do: {day, Base.encode64(der)}
        # A new day gives a new cache key; an empty answer is not kept.
        if map_size(keys) > 0, do: :persistent_term.put(cache, keys)
        keys

      keys ->
        keys
    end
  end

  @doc "How many days from `from` on have a key, in a row."
  def days_ahead(from) do
    Stream.iterate(from, &Date.add(&1, 1))
    |> Enum.take_while(&File.exists?(Path.join(dir(), Date.to_iso8601(&1) <> ".pem")))
    |> length()
  end

  @doc """
  Say, in the log, when the door is about to run out of day keys: with none
  for today nobody can enter, and keys must be made and published ahead.
  """
  def check(today \\ Date.utc_today()) do
    require Logger

    case days_ahead(today) do
      0 -> Logger.error("den: the door has no key for today (#{today}); nobody can enter. Make keys with `den eval 'Den.Release.keys(90, ...)'` and publish them.")
      n when n < 14 -> Logger.warning("den: the door has day keys for #{n} more days; make and publish more before #{Date.add(today, n)}.")
      _ -> :ok
    end
  end

  @doc "Make the day's key if it does not exist."
  def make(day) do
    path = Path.join(dir(), day <> ".pem")

    unless File.exists?(path) do
      File.mkdir_p!(dir())
      key = :public_key.generate_key({:rsa, 2048, 65_537})
      File.write!(path, :public_key.pem_encode([:public_key.pem_entry_encode(:RSAPrivateKey, key)]))
      File.chmod!(path, 0o600)
    end

    :ok
  end

  @doc """
  Sign a blinded message with the day's key: m^d mod n, checked. Through
  OpenSSL's RSA (no padding), which uses the key's CRT parameters and blinds
  the exponentiation, so its timing does not depend on the private exponent;
  a plain modular power would, on inputs anyone may choose.
  """
  def blind_sign(day, blinded) do
    with {:ok, {:RSAPrivateKey, _, n, e, _, _, _, _, _, _, _} = key} <- private(day),
         k = byte_size(:binary.encode_unsigned(n)),
         true <- byte_size(blinded) == k or {:error, :wrong_length},
         m = :binary.decode_unsigned(blinded),
         true <- m < n or {:error, :out_of_range} do
      s = :public_key.encrypt_private(blinded, key, rsa_padding: :rsa_no_padding)
      # RFC 9474 §4.3: check the signature before it leaves.
      if :crypto.mod_pow(s, e, n) == :binary.encode_unsigned(m) do
        {:ok, pad(s, k)}
      else
        {:error, :fault}
      end
    else
      :error -> {:error, :no_key}
      {:error, _} = err -> err
      false -> {:error, :bad}
    end
  end

  @doc "Whether `sig` is the day key's signature on (randomizer ‖ msg)."
  def valid?(day, msg, randomizer, sig) do
    case private(day) do
      {:ok, key} ->
        :public_key.verify(randomizer <> msg, :sha384, sig, public_of(key), rsa_padding: :rsa_pkcs1_pss_padding, rsa_pss_saltlen: @salt)

      :error ->
        false
    end
  end

  defp pad(s, k) when byte_size(s) < k, do: :binary.copy(<<0>>, k - byte_size(s)) <> s
  defp pad(s, _), do: s
end
