import Config

# config/runtime.exs is executed for all environments, including
# during releases. It is executed after compilation and before the
# system starts, so it is typically used to load production configuration
# and secrets from environment variables or elsewhere. Do not define
# any compile-time configuration in here, as it won't be applied.
# The block below contains prod specific runtime configuration.

# ## Using releases
#
# If you use `mix release`, you need to explicitly enable the server
# by passing the PHX_SERVER=true when you start it:
#
#     PHX_SERVER=true bin/den start
#
# Alternatively, you can use `mix phx.gen.release` to generate a `bin/server`
# script that automatically sets the env var above.
if System.get_env("PHX_SERVER") do
  config :den, DenWeb.Endpoint, server: true
end

config :den, DenWeb.Endpoint, http: [port: String.to_integer(System.get_env("PORT", "4000"))]

# Who may enter, and the rooms every member sees. Each is optional; unset, the
# compiled config (config.exs) stands.
list = fn v -> String.split(v, ",", trim: true) |> Enum.map(&String.trim/1) end
module = fn name -> Module.concat([String.trim(name)]) end

if v = System.get_env("DEN_GATE"), do: config(:den, gate: module.(v))

if v = System.get_env("DEN_ENTRY") do
  ways = for w <- list.(v), do: Map.fetch!(%{"invite" => :invite, "approval" => :approval, "external" => :external}, w)
  config :den, entry: ways
end

if v = System.get_env("DEN_ISSUER_TOKEN"), do: config(:den, issuer_token: v)

# "general:General,news:News"
if v = System.get_env("DEN_ROOMS") do
  config :den, rooms: for(r <- list.(v), [id, title] = String.split(r, ":", parts: 2), do: {id, title})
end

if v = System.get_env("DEN_EXTRA_CHILDREN"), do: config(:den, extra_children: Enum.map(list.(v), module))

# How many days of day keys the den keeps ahead itself (0: the host makes them).
if v = System.get_env("DEN_KEYS_AHEAD"), do: config(:den, keys_ahead: String.to_integer(v))

if config_env() == :prod do
  database_path =
    System.get_env("DATABASE_PATH") ||
      raise """
      environment variable DATABASE_PATH is missing.
      For example: /etc/den/den.db
      """

  # Where the day keys are, and which pages may call the den.
  config :den,
    keys_dir: System.get_env("DEN_KEYS_DIR", "/var/lib/den/keys"),
    # Other sites whose pages may use this den (comma-separated https
    # origins); the den's own page always may.
    origins: String.split(System.get_env("DEN_ORIGINS", ""), ",", trim: true),
    # Origins the den's own page may reach besides the den: where its day keys
    # are published, if not on the den itself (comma-separated).
    page_connect: String.split(System.get_env("DEN_PAGE_CONNECT", ""), ",", trim: true),
    voice_public_only: System.get_env("DEN_VOICE_PUBLIC_ONLY") in ["1", "true"]

  config :den, Den.Repo,
    database: database_path,
    pool_size: String.to_integer(System.get_env("POOL_SIZE") || "5")

  # The secret key base is used to sign/encrypt cookies and other secrets.
  # A default value is used in config/dev.exs and config/test.exs but you
  # want to use a different value for prod and you most likely don't want
  # to check this value into version control, so we use an environment
  # variable instead.
  secret_key_base =
    System.get_env("SECRET_KEY_BASE") ||
      raise """
      environment variable SECRET_KEY_BASE is missing.
      You can generate one by calling: mix phx.gen.secret
      """

  host = System.get_env("PHX_HOST") || "example.com"

  config :den, :dns_cluster_query, System.get_env("DNS_CLUSTER_QUERY")

  config :den, DenWeb.Endpoint,
    url: [host: host, port: 443, scheme: "https"],
    http: [
      # Enable IPv6 and bind on all interfaces.
      # Set it to  {0, 0, 0, 0, 0, 0, 0, 1} for local network only access.
      # See the documentation on https://hexdocs.pm/bandit/Bandit.html#t:options/0
      # for details about using IPv6 vs IPv4 and loopback vs public addresses.
      # The den listens on the host's loopback only: nginx is its door to
      # the world (DEN_IP=0.0.0.0 to change that).
      ip: System.get_env("DEN_IP", "127.0.0.1") |> String.to_charlist() |> :inet.parse_address() |> elem(1)
    ],
    secret_key_base: secret_key_base

  # ## SSL Support
  #
  # To get SSL working, you will need to add the `https` key
  # to your endpoint configuration:
  #
  #     config :den, DenWeb.Endpoint,
  #       https: [
  #         ...,
  #         port: 443,
  #         cipher_suite: :strong,
  #         keyfile: System.get_env("SOME_APP_SSL_KEY_PATH"),
  #         certfile: System.get_env("SOME_APP_SSL_CERT_PATH")
  #       ]
  #
  # The `cipher_suite` is set to `:strong` to support only the
  # latest and more secure SSL ciphers. This means old browsers
  # and clients may not be supported. You can set it to
  # `:compatible` for wider support.
  #
  # `:keyfile` and `:certfile` expect an absolute path to the key
  # and cert in disk or a relative path inside priv, for example
  # "priv/ssl/server.key". For all supported SSL configuration
  # options, see https://hexdocs.pm/plug/Plug.SSL.html#configure/1
  #
  # We also recommend setting `force_ssl` in your config/prod.exs,
  # ensuring no data is ever sent via http, always redirecting to https:
  #
  #     config :den, DenWeb.Endpoint,
  #       force_ssl: [hsts: true]
  #
  # Check `Plug.SSL` for all available options in `force_ssl`.
end
