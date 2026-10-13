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
    {:ok, nil, {:continue, :keys}}
  end

  # The day keys (made in Den.Application before the den starts), checked.
  @impl true
  def handle_continue(:keys, s) do
    Den.Keys.check()
    {:noreply, s}
  end

  @impl true
  def handle_info(:sweep, s) do
    run()
    # Once an hour: does the door still have day keys ahead?
    if rem(System.os_time(:second), 3600) < div(Application.get_env(:den, :sweep_ms, 60_000), 1000), do: (Den.Keys.keep(); Den.Keys.check())
    schedule()
    {:noreply, s}
  end

  def run do
    if :ets.whereis(DenWeb.RateLimit) != :undefined, do: DenWeb.RateLimit.sweep()
    for {room, epoch, seq} <- Den.Rooms.undo_unannounced(), do: DenWeb.Endpoint.broadcast("room:" <> room, "rolled_back", %{epoch: epoch, seq: seq})
    {by_room, ended} = Den.Rooms.sweep()
    Den.Mod.sweep()
    Den.Mod.slow_sweep()

    for {room, keys} <- by_room do
      DenWeb.Endpoint.broadcast("room:" <> room, "expired", %{keys: keys})
    end

    for key <- ended, do: DenWeb.Endpoint.broadcast("device:" <> key, "disconnect", %{})
    checkpoint()
    {by_room, ended}
  end

  # What was deleted leaves the write-ahead log too (secure_delete has
  # overwritten it in the database itself).
  defp checkpoint do
    Den.Repo.query("PRAGMA wal_checkpoint(TRUNCATE)")
  rescue
    _ -> :ok
  end

  defp schedule, do: Process.send_after(self(), :sweep, Application.get_env(:den, :sweep_ms, 60_000))
end
