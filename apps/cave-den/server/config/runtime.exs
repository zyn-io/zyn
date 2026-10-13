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
  known = %{"invite" => :invite, "approval" => :approval, "external" => :external}

  ways =
    for w <- list.(v) do
      Map.get(known, w) || raise("DEN_ENTRY: #{inspect(w)} is not a way in; use invite, approval or external, comma-separated")
    end

  config :den, entry: ways
end

if v = System.get_env("DEN_ISSUER_TOKEN") do
  byte_size(v) >= 32 || raise "DEN_ISSUER_TOKEN must be at least 32 characters (openssl rand -hex 32)"
  config :den, issuer_token: v
end

# The public address of a den behind 1:1 NAT, for voice (Den.Voice.public_ip/0).
if v = System.get_env("DEN_PUBLIC_IP") do
  case :inet.parse_address(String.to_charlist(String.trim(v))) do
    {:ok, ip} -> config :den, public_ip: ip
    _ -> raise "DEN_PUBLIC_IP: #{inspect(v)} is not an IP address"
  end
end

# "general:General,news:News"
if v = System.get_env("DEN_ROOMS") do
  config :den, rooms: for(r <- list.(v), [id, title] = String.split(r, ":", parts: 2), do: {id, title})
end

if v = System.get_env("DEN_EXTRA_CHILDREN"), do: config(:den, extra_children: Enum.map(list.(v), module))

# The den's limits (Den.Limits), as "files=100,file_bytes=100000000".
if v = System.get_env("DEN_LIMITS") do
  limits =
    for pair <- list.(v), into: %{} do
      with [k, n] <- String.split(pair, "=", parts: 2),
           {n, ""} <- Integer.parse(String.trim(n)),
           key when is_atom(key) <- Enum.find([:message_bytes, :file_max, :file_bytes, :files, :room_members, :invites, :inbox, :history_bytes, :min_free_bytes], &(Atom.to_string(&1) == String.trim(k))) do
        {key, n}
      else
        _ -> raise "DEN_LIMITS: #{inspect(pair)} is not name=number (message_bytes, file_max, file_bytes, files, room_members, invites, inbox, history_bytes, min_free_bytes)"
      end
    end

  config :den, limits: limits
end

if v = System.get_env("DEN_RETENTION_DAYS"), do: config(:den, retention_days: String.to_integer(v))

# How many days of day keys the den keeps ahead itself (0: the host makes them).
if v = System.get_env("DEN_KEYS_AHEAD"), do: config(:den, keys_ahead: String.to_integer(v))

if v = System.get_env("DEN_TOMORROW_FROM") do
  h = String.to_integer(v)
  h in 0..23 || raise "DEN_TOMORROW_FROM: the UTC hour (0 to 23) from which the door signs tomorrow's tokens"
  config :den, tomorrow_from_hour: h
end

if config_env() == :prod do
  database_path = System.get_env("DATABASE_PATH", "/var/lib/den/den.db")

  # Where the day keys are, and which pages may call the den.
  config :den,
    keys_dir: System.get_env("DEN_KEYS_DIR", "/var/lib/den/keys"),
    # Other sites whose pages may use this den (comma-separated https
    # origins); the den's own page always may.
    origins: System.get_env("DEN_ORIGINS", "") |> String.split(",", trim: true) |> Enum.map(&String.trim/1),
    # Origins the den's own page may reach besides the den: where its day keys
    # are published, if not on the den itself (comma-separated).
    page_connect: System.get_env("DEN_PAGE_CONNECT", "") |> String.split(",", trim: true) |> Enum.map(&String.trim/1),
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

  # The den's page checks its socket's origin against this: a wrong host
  # would shut its own page out, so there is no default.
  host =
    System.get_env("PHX_HOST") ||
      raise "environment variable PHX_HOST is missing: the den's public host name, e.g. chat.example.org"

  config :den, DenWeb.Endpoint,
    url: [host: host, port: 443, scheme: "https"],
    http: [
      # The den listens on the host's loopback only, behind a TLS proxy
      # (Caddy, nginx) on the same machine (DEN_IP=0.0.0.0 to change that).
      ip: System.get_env("DEN_IP", "127.0.0.1") |> String.to_charlist() |> :inet.parse_address() |> elem(1)
    ],
    secret_key_base: secret_key_base

end
