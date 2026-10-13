defmodule Den.MixProject do
  use Mix.Project

  def project do
    [
      app: :den,
      version: "0.1.0",
      elixir: "~> 1.15",
      elixirc_paths: elixirc_paths(Mix.env()) ++ extra("lib"),
      test_paths: ["test"] ++ extra("test"),
      start_permanent: Mix.env() == :prod,
      aliases: aliases(),
      deps: deps(),
      listeners: [Phoenix.CodeReloader]
    ]
  end

  # Configuration for the OTP application.
  #
  # Type `mix help compile.app` for more information.
  def application do
    [
      mod: {Den.Application, []},
      extra_applications: [:logger, :runtime_tools, :inets, :ssl, :public_key, :crypto]
    ]
  end

  def cli do
    [
      preferred_envs: [precommit: :test]
    ]
  end

  # A host's own modules (a gate, add-on processes) in extra/lib, and their
  # tests in extra/test: compiled in when the folder is there, never part of
  # the den itself.
  defp extra(dir), do: if(File.dir?(Path.join("extra", dir)), do: [Path.join("extra", dir)], else: [])

  # Specifies which paths to compile per environment.
  defp elixirc_paths(:test), do: ["lib", "test/support"]
  defp elixirc_paths(_), do: ["lib"]

  # Specifies your project dependencies.
  #
  # Type `mix help deps` for examples and options.
  defp deps do
    [
      {:phoenix, "~> 1.8.5"},
      {:phoenix_ecto, "~> 4.5"},
      {:ecto_sql, "~> 3.13"},
      {:ecto_sqlite3, ">= 0.0.0"},
      {:telemetry_metrics, "~> 1.0"},
      {:telemetry_poller, "~> 1.0"},
      {:jason, "~> 1.2"},
      {:dns_cluster, "~> 0.2.0"},
      {:bandit, "~> 1.5"},
      # Voice rooms: a selective forwarding unit in Elixir.
      {:ex_webrtc, "~> 0.17.0"},
      # The rooms' public MLS state (native/den_public): commits checked before ordering.
      {:rustler, "~> 0.38.0", runtime: false}
    ]
  end

  # Aliases are shortcuts or tasks specific to the current project.
  # For example, to install project dependencies and perform other setup tasks, run:
  #
  #     $ mix setup
  #
  # See the documentation for `Mix` for more info on aliases.
  defp aliases do
    [
      setup: ["deps.get", "ecto.setup"],
      "ecto.setup": ["ecto.create", "ecto.migrate", "run priv/repo/seeds.exs"],
      "ecto.reset": ["ecto.drop", "ecto.setup"],
      test: ["ecto.create --quiet", "ecto.migrate --quiet", "test"],
      precommit: ["compile --warnings-as-errors", "deps.unlock --unused", "format", "test"]
    ]
  end
end
