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
    unless File.exists?(path), do: File.write!(path, bytes)
    Repo.insert_all("blobs", [%{id: id, size: byte_size(bytes), inserted_at: Den.Rooms.now()}], on_conflict: :nothing)
    {:ok, id}
  end

  def put(_), do: {:error, :size}

  def path(id) do
    if id =~ ~r/^[0-9a-f]{64}$/ and Repo.exists?(from b in "blobs", where: b.id == ^id), do: {:ok, Path.join(dir(), id)}, else: :error
  end
end
