defmodule Mix.Tasks.Den.Keys do
  @shortdoc "Make the door's day keys ahead, and write the public ones to publish"
  @moduledoc """
      mix den.keys [--days 45] [--out den-keys.json]

  Makes a private key for each day from today that has none (in `keys_dir`),
  and writes every public key from today on to `--out`: the file the host
  publishes, the same for everyone, so the door cannot tell members apart by
  their keys.
  """
  use Mix.Task

  @impl true
  def run(args) do
    {opts, _, _} = OptionParser.parse(args, strict: [days: :integer, out: :string])
    Mix.Task.run("app.config")
    days = opts[:days] || 45
    today = Date.utc_today()
    for d <- 0..(days - 1), do: Den.Keys.make(Date.to_iso8601(Date.add(today, d)))
    keys = Den.Keys.published(today, days)
    out = opts[:out] || "den-keys.json"
    File.write!(out, Jason.encode!(%{suite: "RSABSSA-SHA384-PSS-Randomized", keys: keys}, pretty: true))
    Mix.shell().info("#{map_size(keys)} day keys from #{today}; public ones in #{out}")
  end
end
