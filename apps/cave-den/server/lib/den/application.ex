defmodule Den.Application do
  # See https://hexdocs.pm/elixir/Application.html
  # for more information on OTP Applications
  @moduledoc false

  use Application

  @impl true
  def start(_type, _args) do
    # No key, private room or payload in any log line.
    Den.LogRedact.install()
    # The door's day keys, made before the den answers anyone.
    Den.Keys.keep()
    DenWeb.RateLimit.setup()
    # ex_ice listens for mDNS answers on 0.0.0.0:5353 and caches whatever
    # arrives. A den on the open internet resolves no .local names (it refuses
    # those candidates), so that listener goes.
    with {:ok, sup} <- :application.get_supervisor(:ex_ice), do: Supervisor.terminate_child(sup, ExICE.Priv.MDNS.Resolver)

    children = [
      DenWeb.Telemetry,
      Den.Repo,
      {Ecto.Migrator,
       repos: Application.fetch_env!(:den, :ecto_repos), skip: skip_migrations?()},
      {Phoenix.PubSub, name: Den.PubSub},
      # The configured rooms, made once the database is there.
      {Task, &Den.Rooms.seed/0},
      Den.Sweeper,
      {Registry, keys: :unique, name: Den.Voice.Registry},
      # Which devices are connected to which room now (Den.Rooms.report/4).
      {Registry, keys: :duplicate, name: Den.Online},
      # Every voice connection, for the den-wide cap.
      {Registry, keys: :duplicate, name: Den.Voice.Peers},
      {DynamicSupervisor, name: Den.Voice.Sup, strategy: :one_for_one}
    ] ++
      # A host's add-on processes (`DEN_EXTRA_CHILDREN`), compiled in from extra/.
      Application.get_env(:den, :extra_children, []) ++
      [
        # Start to serve requests, typically the last entry
        DenWeb.Endpoint
      ]

    # See https://hexdocs.pm/elixir/Supervisor.html
    # for other strategies and supported options
    opts = [strategy: :one_for_one, name: Den.Supervisor]
    Supervisor.start_link(children, opts)
  end

  # Tell Phoenix to update the endpoint configuration
  # whenever the application is updated.
  @impl true
  def config_change(changed, _new, removed) do
    DenWeb.Endpoint.config_change(changed, removed)
    :ok
  end

  defp skip_migrations?() do
    # By default, sqlite migrations are run when using a release
    System.get_env("RELEASE_NAME") == nil
  end
end
