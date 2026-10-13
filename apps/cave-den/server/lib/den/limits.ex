defmodule Den.Limits do
  @moduledoc """
  What one device may put on the den in a day, and when the den stops taking
  anything: set under `config :den, :limits` (each key optional).

  * `:message_bytes` — bytes of messages relayed, per device per day (20 MB)
  * `:file_max` — bytes of one file as stored, encrypted (100 MB)
  * `:file_bytes` — bytes of files stored, per device per day (500 MB)
  * `:files` — files stored, per device per day (200)
  * `:room_members` — members in one room (500; up to 1000)
  * `:invites` — invitations sent, per device per day (200)
  * `:inbox` — Welcomes waiting for one device at once (100)
  * `:history_bytes` — ciphertext in one history reply (4 MB)
  * `:min_free_bytes` — free disk below which no message or file is taken (2 GB)
  """

  @defaults %{
    message_bytes: 20_000_000,
    file_max: 100_000_000,
    file_bytes: 500_000_000,
    files: 200,
    room_members: 500,
    invites: 200,
    inbox: 100,
    history_bytes: 4_000_000,
    min_free_bytes: 2_000_000_000
  }

  # A map or a keyword list in config; DEN_LIMITS="files=100,file_bytes=…".
  def get(k), do: Map.get(Map.new(Application.get_env(:den, :limits, %{})), k, Map.fetch!(@defaults, k))

  @doc """
  Whether the disk the den writes to has room: read with `df` at most once a
  minute (cached), and true when it cannot tell.
  """
  def disk_ok? do
    now = System.monotonic_time(:second)

    case :persistent_term.get({__MODULE__, :disk}, nil) do
      {at, ok} when now - at < 60 ->
        ok

      _ ->
        ok = free_bytes() |> then(&(is_nil(&1) or &1 >= get(:min_free_bytes)))
        :persistent_term.put({__MODULE__, :disk}, {now, ok})
        ok
    end
  end

  defp free_bytes do
    dir = Den.Blobs.dir()
    File.mkdir_p(dir)

    with {out, 0} <- System.cmd("df", ["-Pk", dir], stderr_to_stdout: true),
         [_, line | _] <- String.split(out, "\n", trim: true),
         [_, _, _, avail | _] <- String.split(line),
         {kb, _} <- Integer.parse(avail) do
      kb * 1024
    else
      _ -> nil
    end
  rescue
    _ -> nil
  end
end
