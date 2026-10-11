# This file is responsible for configuring your application
# and its dependencies with the aid of the Config module.
#
# This configuration file is loaded before any dependency and
# is restricted to this project.

# General application configuration
import Config

config :den,
  ecto_repos: [Den.Repo],
  generators: [timestamp_type: :utc_datetime]

# Configure the endpoint
config :den, DenWeb.Endpoint,
  url: [host: "localhost"],
  adapter: Bandit.PhoenixAdapter,
  render_errors: [
    formats: [json: DenWeb.ErrorJSON],
    layout: false
  ],
  pubsub_server: Den.PubSub,
  live_view: [signing_salt: "+rMCX0rh"]

# Configure Elixir's Logger
config :logger, :default_formatter,
  format: "$time $metadata[$level] $message\n",
  metadata: [:request_id]

# Use Jason for JSON parsing in Phoenix
config :phoenix, :json_library, Jason

# The den (DECISIONS §184).
config :den,
  rooms: [{"general", "General"}, {"diggers", "Diggers"}, {"lookouts", "Lookouts"}, {"lanterns", "Lanterns"}],
  collection: "024277b607ce4edbf78fe2855aba562096d00264729168bd0ddea71bdf5b5f1e",
  node_url: "http://127.0.0.1:8599",
  keys_dir: "priv/keys",
  origins: ["https://cavebro.win"],
  retention_days: 30

# Import environment specific config. This must remain at the bottom
# of this file so it overrides the configuration defined above.
import_config "#{config_env()}.exs"

