defmodule DenWeb.BlobController do
  @moduledoc """
  Encrypted files in and out, for admitted devices only (the session the door
  gave, as a bearer token). The bytes are ciphertext; the key is in an MLS
  message the den cannot read.
  """
  use DenWeb, :controller

  defp device(conn) do
    with ["Bearer " <> s] <- get_req_header(conn, "authorization"),
         {:ok, %{key: key}} <- Phoenix.Token.verify(DenWeb.Endpoint, "den", s, max_age: 3 * 86_400),
         %DateTime{} = until <- Den.Rooms.device_until(key),
         :gt <- DateTime.compare(until, DateTime.utc_now()),
         false <- Den.Mod.out?(key) do
      {:ok, key}
    else
      _ -> :error
    end
  end

  # A file is refused by its declared size before a byte is read, its bytes
  # counted against the day's allowance up front, and streamed to disk in
  # 1 MB pieces while hashed, never held whole in memory.
  def create(conn, _) do
    with {:ok, key} <- device(conn),
         {:ok, size} <- declared(conn),
         true <- Den.Limits.disk_ok?() || {:error, :disk},
         :ok <- Den.Rooms.quota(key, "file", Den.Limits.get(:files)),
         :ok <- Den.Rooms.quota(key, "file_bytes", Den.Limits.get(:file_bytes), size),
         {:ok, tmp, hash, conn} <- stream(conn, size),
         {:ok, id} <- Den.Blobs.put_file(tmp, hash, size) do
      json(conn, %{id: id})
    else
      :error -> conn |> put_status(401) |> json(%{error: "enter the den first"})
      {:error, :quota} -> conn |> put_status(429) |> json(%{error: "this device has sent enough files today"})
      {:error, :disk} -> conn |> put_status(507) |> json(%{error: "the den is full for now; try again later"})
      {:error, :size} -> conn |> put_status(413) |> json(%{error: "a file is at most #{div(Den.Blobs.max(), 1_000_000)} MB here, and its length is sent first"})
      _ -> conn |> put_status(400) |> json(%{error: "could not read the file"})
    end
  end

  defp declared(conn) do
    with [v] <- get_req_header(conn, "content-length"),
         {n, ""} when n > 0 <- Integer.parse(v),
         true <- n <= Den.Blobs.max() do
      {:ok, n}
    else
      _ -> {:error, :size}
    end
  end

  defp stream(conn, size) do
    tmp = Den.Blobs.part()
    {:ok, io} = File.open(tmp, [:write, :binary, :raw])

    try do
      read(conn, io, :crypto.hash_init(:sha256), 0, size)
    after
      File.close(io)
    end
    |> case do
      {:ok, hash, conn} -> {:ok, tmp, hash, conn}
      other -> File.rm(tmp) && other
    end
  end

  defp read(conn, io, h, n, size) do
    case Plug.Conn.read_body(conn, length: 1_000_000, read_length: 1_000_000) do
      {status, chunk, conn} when status in [:ok, :more] ->
        n = n + byte_size(chunk)

        if n > size do
          {:error, :size}
        else
          :ok = IO.binwrite(io, chunk)
          h = :crypto.hash_update(h, chunk)
          cond do
            status == :more -> read(conn, io, h, n, size)
            n == size -> {:ok, :crypto.hash_final(h), conn}
            true -> {:error, :size}
          end
        end

      other ->
        other
    end
  end

  def show(conn, %{"id" => id}) do
    with {:ok, _} <- device(conn),
         {:ok, path} <- Den.Blobs.path(id) do
      conn |> put_resp_content_type("application/octet-stream") |> put_resp_header("cache-control", "private, max-age=86400") |> send_file(200, path)
    else
      _ -> conn |> put_status(404) |> json(%{error: "no such file (or you have not entered)"})
    end
  end
end
