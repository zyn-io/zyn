defmodule Den.Sweeper do
  @moduledoc """
  Every minute: devices whose day ended are named to each room they were in
  (so its members' devices take them out of the group), their connections are
  closed, and ciphertext and door records past their time are dropped.
  """
  use GenServer

  def start_link(_), do: GenServer.start_link(__MODULE__, nil, name: __MODULE__)

  @impl true
  def init(_) do
    schedule()
    {:ok, nil}
  end

  @impl true
  def handle_info(:sweep, s) do
    run()
    schedule()
    {:noreply, s}
  end

  def run do
    {by_room, ended} = Den.Rooms.sweep()

    for {room, keys} <- by_room do
      DenWeb.Endpoint.broadcast("room:" <> room, "expired", %{keys: keys})
    end

    for key <- ended, do: DenWeb.Endpoint.broadcast("device:" <> key, "disconnect", %{})
    {by_room, ended}
  end

  defp schedule, do: Process.send_after(self(), :sweep, Application.get_env(:den, :sweep_ms, 60_000))
end
