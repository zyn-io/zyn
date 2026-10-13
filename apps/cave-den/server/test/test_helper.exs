# The rooms' public-state NIF loads with its module: once, before any test
# waits on a reply that needs it.
Code.ensure_loaded!(Den.Public)
ExUnit.start()
Ecto.Adapters.SQL.Sandbox.mode(Den.Repo, :manual)
