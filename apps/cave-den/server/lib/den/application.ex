defmodule Den.Application do
  # See https://hexdocs.pm/elixir/Application.html
  # for more information on OTP Applications
  @moduledoc false

  use Application

  @impl true
  def start(_type, _args) do
    children = [
      DenWeb.Telemetry,
      Den.Repo,
      {Ecto.Migrator,
       repos: Application.fetch_env!(:den, :ecto_repos), skip: skip_migrations?()},
      {DNSCluster, query: Application.get_env(:den, :dns_cluster_query) || :ignore},
      {Phoenix.PubSub, name: Den.PubSub},
      # The configured rooms, made once the database is there.
      {Task, &Den.Rooms.seed/0},
      Den.Sweeper,
      {Registry, keys: :unique, name: Den.Voice.Registry},
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
