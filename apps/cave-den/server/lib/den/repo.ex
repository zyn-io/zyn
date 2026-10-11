defmodule Den.Repo do
  use Ecto.Repo,
    otp_app: :den,
    adapter: Ecto.Adapters.SQLite3
end
