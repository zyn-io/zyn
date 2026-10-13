import Config

# Configure your database
#
# The MIX_TEST_PARTITION environment variable can be used
# to provide built-in test partitioning in CI environment.
# Run `mix help test` for more information.
config :den, Den.Repo,
  database: Path.expand("../den_test.db", __DIR__),
  pool_size: 5,
  pool: Ecto.Adapters.SQL.Sandbox

# We don't run a server during test. If one is required,
# you can enable the server option below.
config :den, DenWeb.Endpoint,
  http: [ip: {127, 0, 0, 1}, port: 4002],
  secret_key_base: "f9yk2jtYhxlQ8MrZCjmcMrePwJC9QOiK8ua9J4cOEyEh2ePUiHWBB48J1nLjcYTR",
  server: false

# Print only warnings and errors during test
config :logger, level: :warning

# Initialize plugs at runtime for faster test compilation
config :phoenix, :plug_init_mode, :runtime

# Sort query params output of verified routes for robust url comparisons
config :phoenix,
  sort_verified_routes_query_params: true

config :den, keys_dir: Path.expand("../tmp/test-keys", __DIR__), origins: ["http://localhost:4180"], sweep_ms: 3_600_000,
  entry: [:invite, :approval, :external], issuer_token: String.duplicate("t", 40),
  rooms: [{"general", "General"}, {"news", "News"}], keys_ahead: 0, mod_cache: false, door_per_minute: 1_000_000, ask_per_hour: 1_000_000

