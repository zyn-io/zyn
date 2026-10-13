defmodule DenWeb.Throttle do
  @moduledoc """
  How often one channel may send each kind of event, in 10-second windows,
  counted in the channel's own process: typing, history pages, GroupInfo,
  state reads, reports and invitations published are cheap to send and not
  free to answer.
  """
  @window 10

  @limits %{"typing" => 5, "history" => 20, "state" => 20, "info" => 5, "create" => 5, "report" => 5, "relay" => 60, "invite" => 10, "publish" => 5, "taken" => 20, "claim" => 10, "open_room" => 5, "private_room" => 10, "offer" => 5, "ice" => 40, "leave" => 5, "relay_join" => 5, "frame" => 600}

  @doc "Whether this channel may handle one more `event` now."
  def allow?(event) do
    max = Map.get(@limits, event, 20)
    slot = div(System.monotonic_time(:second), @window)

    case Process.get({__MODULE__, event}) do
      {^slot, n} when n >= max -> false
      {^slot, n} -> put(event, {slot, n + 1})
      _ -> put(event, {slot, 1})
    end
  end

  defp put(event, v) do
    Process.put({__MODULE__, event}, v)
    true
  end

  @doc "The reply for an event over its limit."
  def refused, do: {:error, %{reason: "slow down: too many of these in a few seconds"}}
end
