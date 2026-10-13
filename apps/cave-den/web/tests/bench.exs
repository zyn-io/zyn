# The den's side of a room's size: the time its native library takes to start
# tracking a room from a GroupInfo, to check and apply a commit, and to list
# the members, and the size of the public state it keeps per room. Reads the
# fixtures web/tests/bench.mjs wrote.
#
#   cd server && mix run ../web/tests/bench.exs
dir = Path.join(System.tmp_dir!(), "den-bench")
time = fn f -> {us, r} = :timer.tc(f); {Float.round(us / 1000, 1), r} end

for file <- Path.wildcard(Path.join(dir, "*.json")) |> Enum.sort_by(&String.to_integer(Path.basename(&1, ".json"))) do
  n = Path.basename(file, ".json")
  f = Jason.decode!(File.read!(file))
  {track_ms, {:ok, {state, room, _}}} = time.(fn -> Den.Public.track(Base.decode64!(f["info"])) end)
  runs = for _ <- 1..5 do
    {ms, {:ok, {after_state, _, _}}} = time.(fn -> Den.Public.commit(state, Base.decode64!(f["join"])) end)
    {ms, after_state}
  end
  commit_ms = runs |> Enum.map(&elem(&1, 0)) |> Enum.sort() |> Enum.at(2)
  {_, after_state} = hd(runs)
  {members_ms, {:ok, keys}} = time.(fn -> Den.Public.members(after_state, room) end)
  {info_ms, {:ok, ok}} = time.(fn -> Den.Public.info_matches(after_state, Base.decode64!(f["info2"])) end)
  IO.puts(Jason.encode!(%{members: n, listed: length(keys), track_ms: track_ms, commit_ms: commit_ms, members_ms: members_ms, info_check_ms: info_ms, info_matches: ok, state_kb: div(byte_size(after_state), 1024)}))
end
