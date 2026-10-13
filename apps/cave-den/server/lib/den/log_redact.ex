defmodule Den.LogRedact do
  @moduledoc """
  Every log event, crash reports included, before any handler writes it:
  device keys and other 64-hex values, private room ids and long encoded
  payloads are replaced, so a crash in a channel does not write who was in
  which private room, or what they sent, to the server's log.
  """

  def install do
    :logger.remove_primary_filter(:den_redact)
    :logger.add_primary_filter(:den_redact, {&__MODULE__.filter/2, []})
  end

  def filter(%{msg: msg} = event, _) do
    %{event | msg: {:string, redact(text(msg, event))}}
  rescue
    _ -> %{event | msg: {:string, "(a log line the den could not redact was dropped)"}}
  end

  defp text({:string, s}, _), do: IO.chardata_to_string(s)
  defp text({:report, r}, %{meta: %{report_cb: cb}}) when is_function(cb, 1), do: cb.(r) |> then(fn {f, a} -> :io_lib.format(f, a) end) |> IO.chardata_to_string()
  defp text({:report, r}, _), do: inspect(r, limit: 50, printable_limit: 500)
  defp text({f, a}, _), do: :io_lib.format(f, a) |> IO.chardata_to_string()

  @doc "The text with keys, private room ids and payloads taken out."
  def redact(t) do
    t
    |> String.replace(~r/p-[0-9a-f]{32}/, "p-<private>")
    |> String.replace(~r/[0-9a-fA-F]{64,}/, "<hex>")
    |> String.replace(~r/[A-Za-z0-9+\/_-]{120,}={0,2}/, "<bytes>")
  end
end
