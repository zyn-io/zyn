defmodule Den.Release do
  @moduledoc """
  What the release does without Mix: migrate, and make the door's day keys
  ahead and write the public ones to publish.

      bin/den eval 'Den.Release.keys(90, "/var/lib/den/den-keys.json")'
  """

  def migrate do
    Application.load(:den)
    for repo <- Application.fetch_env!(:den, :ecto_repos), do: {:ok, _, _} = Ecto.Migrator.with_repo(repo, &Ecto.Migrator.run(&1, :up, all: true))
  end

  def keys(days, out) do
    Application.load(:den)
    Application.ensure_all_started(:crypto)
    Application.ensure_all_started(:public_key)
    today = Date.utc_today()
    for d <- 0..(days - 1), do: Den.Keys.make(Date.to_iso8601(Date.add(today, d)))
    keys = Den.Keys.published(today, days)
    File.write!(out, Jason.encode!(%{suite: "RSABSSA-SHA384-PSS-Randomized", keys: keys}, pretty: true))
    IO.puts("#{map_size(keys)} day keys from #{today}; public ones in #{out}")
  end
end
