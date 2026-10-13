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
         :gt <- DateTime.compare(until, DateTime.utc_now()) do
      {:ok, key}
    else
      _ -> :error
    end
  end

  def create(conn, _) do
    with {:ok, key} <- device(conn),
         true <- Den.Limits.disk_ok?() || {:error, :disk},
         :ok <- Den.Rooms.quota(key, "file", Den.Limits.get(:files)),
         {:ok, body, conn} <- read_all(conn, ""),
         :ok <- Den.Rooms.quota(key, "file_bytes", Den.Limits.get(:file_bytes), byte_size(body)),
         {:ok, id} <- Den.Blobs.put(body) do
      json(conn, %{id: id})
    else
      :error -> conn |> put_status(401) |> json(%{error: "enter the den first"})
      {:error, :quota} -> conn |> put_status(429) |> json(%{error: "this device has sent enough files today"})
      {:error, :disk} -> conn |> put_status(507) |> json(%{error: "the den is full for now; try again later"})
      {:error, :size} -> conn |> put_status(413) |> json(%{error: "a file is at most 25 MB"})
      _ -> conn |> put_status(400) |> json(%{error: "could not read the file"})
    end
  end

  defp read_all(conn, acc) do
    case Plug.Conn.read_body(conn, length: 1_000_000, read_length: 1_000_000) do
      {:ok, chunk, conn} -> check(acc <> chunk, conn, :ok)
      {:more, chunk, conn} -> check(acc <> chunk, conn, :more)
      other -> other
    end
  end

  defp check(acc, _conn, _) when byte_size(acc) > 25 * 1024 * 1024, do: {:error, :size}
  defp check(acc, conn, :ok), do: {:ok, acc, conn}
  defp check(acc, conn, :more), do: read_all(conn, acc)

  def show(conn, %{"id" => id}) do
    with {:ok, _} <- device(conn),
         {:ok, path} <- Den.Blobs.path(id) do
      conn |> put_resp_content_type("application/octet-stream") |> put_resp_header("cache-control", "private, max-age=86400") |> send_file(200, path)
    else
      _ -> conn |> put_status(404) |> json(%{error: "no such file (or you have not entered)"})
    end
  end
end
