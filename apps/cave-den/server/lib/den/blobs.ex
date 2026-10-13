defmodule Den.Blobs do
  @moduledoc """
  Files, encrypted on the sender's device with a one-time key that travels
  only inside an MLS message. The den keeps the ciphertext under its SHA-256
  for `retention_days`, hands it to admitted devices, and cannot open it.
  """
  import Ecto.Query
  alias Den.Repo

  @max 25 * 1024 * 1024

  def max, do: @max
  def dir, do: Application.get_env(:den, :blobs_dir) || Path.join(Path.dirname(Application.fetch_env!(:den, :keys_dir)), "blobs")

  def put(bytes) when byte_size(bytes) <= @max and byte_size(bytes) > 0 do
    id = :crypto.hash(:sha256, bytes) |> Base.encode16(case: :lower)
    File.mkdir_p!(dir())
    path = Path.join(dir(), id)

    # Written whole or not at all: to a temporary name, then renamed.
    unless File.exists?(path) do
      tmp = path <> ".part-" <> Base.encode16(:crypto.strong_rand_bytes(6), case: :lower)
      File.write!(tmp, bytes)
      File.rename!(tmp, path)
    end

    # Sent again: kept again for its full time.
    now = Den.Rooms.now()
    Repo.insert_all("blobs", [%{id: id, size: byte_size(bytes), inserted_at: now}], on_conflict: [set: [inserted_at: now]], conflict_target: :id)
    {:ok, id}
  end

  def put(_), do: {:error, :size}

  def path(id) do
    if id =~ ~r/\A[0-9a-f]{64}\z/ and Repo.exists?(from b in "blobs", where: b.id == ^id), do: {:ok, Path.join(dir(), id)}, else: :error
  end
end
