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

# SQLite has one writer. A transaction takes the write lock when it begins,
# and waits for it, instead of failing with "Database busy" half way through.
config :den, Den.Repo,
  # Queries are not logged: their parameters are device keys and room ids.
  log: false,
  default_transaction_mode: :immediate,
  # Deleted rows are overwritten, not left in free pages: ciphertext past its
  # 30 days, devices and pass hashes past their day. The write-ahead log is
  # kept small and truncated after each sweep (Den.Sweeper).
  secure_delete: :on,
  journal_size_limit: 67_108_864,
  busy_timeout: 5_000,
  journal_mode: :wal

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

# Never in a log: a pass code, a token or its parts, a device's proof, a
# session (the door's and the socket's parameters).
config :phoenix, :filter_parameters, ["password", "token", "session", "code", "sig", "blinded", "msg", "randomizer", "proof", "blind_sig"]

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

