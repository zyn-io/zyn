defmodule Den.Door do
  @moduledoc """
  The den's door. Two steps, made from different moments so the den cannot
  join them:

  1. `sign/3`: a member's page shows its pass code and a *blinded* token for
     today or, in the day's last hours (from `:tomorrow_from_hour` UTC, 22 by
     default), tomorrow. The door asks the gate (`Den.Gate`) whether the pass
     is live, counts at most #{3} tokens per pass per day, and signs the
     blinded token with the day's key. It sees the pass, never the token.
     Tomorrow's token is signed only late in the day so that a pass the host
     removes before then lets no device in after midnight.
  2. `enter/4`: a device presents an unblinded token, which names its day,
     the device's key and a nonce, on the token's day. The den checks the
     signature against the day key and the device's proof that it holds the
     key, spends the token, and admits the device until the day ends. It sees
     the device, never the pass.
  """
  import Ecto.Query
  alias Den.Repo

  @per_pass_per_day 3
  @grace 3600

  def today, do: (Application.get_env(:den, :today) || fn -> Date.utc_today() end).()
  defp now, do: (Application.get_env(:den, :now) || fn -> DateTime.utc_now() end).()

  @doc "The UTC hour from which the door signs tomorrow's tokens."
  def tomorrow_from, do: Application.get_env(:den, :tomorrow_from_hour, 22)
  defp day_string(d), do: Date.to_iso8601(d)

  @doc "Blind-sign a token for `day` (today or tomorrow) for a live pass."
  def sign(code_hex, day, blinded_hex) do
    today = today()

    with {:ok, code} <- Den.Gate.code(code_hex),
         true <- day in [day_string(today), day_string(Date.add(today, 1))] || {:error, :day},
         true <- day == day_string(today) or now().hour >= tomorrow_from() || {:error, :early},
         {:ok, blinded} <- Base.decode16(blinded_hex || "", case: :lower) |> ok_or(:blinded),
         pass = Den.Gate.id(code),
         {:ok, true} <- live(code),
         :ok <- take_one(Base.encode16(pass, case: :lower), day) do
      Den.Keys.blind_sign(day, blinded)
    else
      :error -> {:error, :code}
      {:error, _} = e -> e
    end
  end

  defp ok_or({:ok, v}, _), do: {:ok, v}
  defp ok_or(:error, what), do: {:error, what}

  defp live(code) do
    case Den.Gate.live?(code) do
      {:ok, true} -> {:ok, true}
      {:ok, false} -> {:error, :not_live}
      {:error, _} -> {:error, :gate}
    end
  end

  defp take_one(pass, day) do
    Repo.transaction(fn ->
      n = Repo.one(from i in "issued", where: i.pass == ^pass and i.day == ^day, select: i.count) || 0
      if n >= @per_pass_per_day, do: Repo.rollback(:limit)
      Repo.insert_all("issued", [%{pass: pass, day: day, count: n + 1}], on_conflict: [set: [count: n + 1]], conflict_target: [:pass, :day])
      :ok
    end)
    |> case do
      {:ok, :ok} -> :ok
      {:error, :limit} -> {:error, :limit}
    end
  end

  @doc """
  Admit the device a token names, on the token's day, if the device proves it
  holds that key: `proof_hex` is its Ed25519 signature over
  `"den-enter-v1|" <> msg`. A token only names a key, and keys are seen
  by every member, so without the proof a member could enter as another
  device and take its inbox. Returns its session.
  """
  def enter(msg, randomizer_hex, sig_hex, proof_hex) do
    today = day_string(today())

    with ["den-token-v1", day, key, nonce] <- String.split(msg || "", "|"),
         true <- (key =~ ~r/\A[0-9a-f]{64}\z/ and nonce =~ ~r/\A[0-9a-f]{32}\z/) || {:error, :token},
         true <- day == today || {:error, :day},
         {:ok, randomizer} <- Base.decode16(randomizer_hex || "", case: :lower) |> ok_or(:token),
         {:ok, sig} <- Base.decode16(sig_hex || "", case: :lower) |> ok_or(:token),
         true <- Den.Keys.valid?(day, msg, randomizer, sig) || {:error, :signature},
         true <- holds_key?(key, msg, proof_hex) || {:error, :proof},
         true <- not Den.Mod.out?(key) || {:error, :out},
         :ok <- spend(msg, day) do
      until = Date.from_iso8601!(day) |> Date.add(1) |> DateTime.new!(~T[00:00:00]) |> DateTime.add(@grace, :second)
      Den.Rooms.admit(key, until)
      {:ok, %{session: Phoenix.Token.sign(DenWeb.Endpoint, "den", %{key: key}), key: key, until: DateTime.to_unix(until)}}
    else
      {:error, _} = e -> e
      _ -> {:error, :token}
    end
  end

  defp holds_key?(key, msg, proof_hex) do
    with {:ok, proof} when byte_size(proof) == 64 <- Base.decode16(proof_hex || "", case: :lower),
         {:ok, pk} <- Base.decode16(key, case: :lower) do
      :crypto.verify(:eddsa, :none, "den-enter-v1|" <> msg, proof, [pk, :ed25519])
    else
      _ -> false
    end
  end

  defp spend(msg, day) do
    id = :crypto.hash(:sha256, msg) |> Base.encode16(case: :lower)

    case Repo.insert_all("spent", [%{token: id, day: day}], on_conflict: :nothing) do
      {1, _} -> :ok
      {0, _} -> {:error, :spent}
    end
  end
end
