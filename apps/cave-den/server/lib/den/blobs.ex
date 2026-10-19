defmodule Den.Blobs do
  @moduledoc """
  Files, encrypted on the sender's device with a one-time key that travels
  only inside an MLS message. The den keeps the ciphertext under its SHA-256
  for `retention_days`, hands it to admitted devices, and cannot open it.
  """
  import Ecto.Query
  alias Den.Repo

  @doc "The most one file may be, encrypted: `:file_max` (Den.Limits)."
  def max, do: Den.Limits.get(:file_max)
  def dir, do: Application.get_env(:den, :blobs_dir) || Path.join(Path.dirname(Application.fetch_env!(:den, :keys_dir)), "blobs")

  def put(bytes) when byte_size(bytes) > 0 do
    if byte_size(bytes) <= max(), do: keep(bytes), else: {:error, :size}
  end

  def put(_), do: {:error, :size}

  defp keep(bytes) do
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

  @doc "A fresh temporary name in the blob folder, for a file being received."
  def part do
    File.mkdir_p!(dir())
    Path.join(dir(), ".part-" <> Base.encode16(:crypto.strong_rand_bytes(8), case: :lower))
  end

  @doc "Keeps a received file (`tmp`, of `size` bytes, SHA-256 `hash`) under its hash."
  def put_file(tmp, hash, size) when size > 0 do
    id = Base.encode16(hash, case: :lower)
    path = Path.join(dir(), id)
    if File.exists?(path), do: File.rm!(tmp), else: File.rename!(tmp, path)
    now = Den.Rooms.now()
    Repo.insert_all("blobs", [%{id: id, size: size, inserted_at: now}], on_conflict: [set: [inserted_at: now]], conflict_target: :id)
    {:ok, id}
  end

  @doc """
  Clears what uploads left half-written: a temporary file nothing has
  written to for `idle` seconds (an upload still coming keeps writing).
  """
  def sweep_parts(idle \\ 3600) do
    now = System.os_time(:second)

    case File.ls(dir()) do
      {:ok, names} ->
        for n <- names, String.contains?(n, ".part-"),
            p = Path.join(dir(), n),
            match?({:ok, %{mtime: t}} when now - t > idle, File.stat(p, time: :posix)),
            do: File.rm(p)

      _ ->
        :ok
    end

    :ok
  end

  def path(id) do
    if id =~ ~r/\A[0-9a-f]{64}\z/ and Repo.exists?(from b in "blobs", where: b.id == ^id), do: {:ok, Path.join(dir(), id)}, else: :error
  end
end
