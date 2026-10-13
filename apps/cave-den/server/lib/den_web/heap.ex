defmodule DenWeb.Heap do
  @moduledoc """
  A ceiling on one connection's memory: a device that will not read what it
  is sent (or sends faster than it is answered) has its process ended past
  it, and reconnects, instead of growing until the den runs out of memory.
  In words (8 bytes each): 64 MB for a socket, 32 MB for a channel.
  """
  def socket, do: cap(8_000_000)
  def channel, do: cap(4_000_000)

  defp cap(words) do
    Process.flag(:max_heap_size, %{size: words, kill: true, error_logger: true})
    :ok
  end
end
