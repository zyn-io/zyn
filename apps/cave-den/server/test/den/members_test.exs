defmodule Den.MembersTest do
  use DenWeb.ConnCase, async: false
  import Ecto.Query
  alias Den.Gate.Members
  alias Den.Repo

  @f Path.expand("../fixtures", __DIR__)
  @issuer String.duplicate("t", 40)

  # A member's device: its own pass code, and the hash it sends.
  defp device_pass do
    code = :crypto.strong_rand_bytes(32)
    {Base.encode16(code, case: :lower), Base.encode16(Members.hash(code), case: :lower)}
  end

  setup do
    Application.put_env(:den, :keys_dir, Path.join(@f, "keys"))
    Application.put_env(:den, :today, fn -> ~D[2026-10-12] end)
    # Late in the day, when the door also signs tomorrow's tokens.
    Application.put_env(:den, :now, fn -> ~U[2026-10-12 22:30:00Z] end)
    blinded = String.trim(File.read!(Path.join(@f, "blinded.hex")))
    on_exit(fn ->
      Application.delete_env(:den, :today)
      Application.delete_env(:den, :now)
      Application.put_env(:den, :entry, [:invite, :approval, :external])
    end)
    %{blinded: blinded}
  end

  # The door's own rules hold for a pass from any way in: three tokens a day,
  # then no more; a pass that left gets none.
  defp door_rules(code, b) do
    for _ <- 1..3, do: assert({:ok, _} = Den.Door.sign(code, "2026-10-12", b))
    assert Den.Door.sign(code, "2026-10-12", b) == {:error, :limit}
  end

  test "an invite lets one device in, once; used, withdrawn or expired it lets nobody in", %{blinded: b} do
    [invite, other] = Members.invite(2, 7)
    {code, pass} = device_pass()
    {code2, pass2} = device_pass()
    assert Den.Door.sign(code, "2026-10-12", b) == {:error, :not_live}
    assert Members.join(invite, pass) == :ok
    door_rules(code, b)
    assert Members.join(invite, pass2) == {:error, :invite}, "an invite is used once"
    assert Den.Door.sign(code2, "2026-10-12", b) == {:error, :not_live}
    assert Members.join(String.duplicate("ab", 32), pass2) == {:error, :invite}, "a made-up invite"
    assert Members.join(other, pass) == {:error, :pass_taken}
    assert Members.join(other, pass2) == :ok, "a refused join did not use the invite up"

    [id] = for i <- Members.invites(), i.id == Base.encode16(:crypto.hash(:sha256, Base.decode16!(invite, case: :lower)), case: :lower), do: i.id
    assert Members.withdraw(String.slice(id, 0, 10)) == {:ok, :member_removed}
    assert Den.Door.sign(code, "2026-10-12", b) == {:error, :not_live}, "withdrawing the invite ends its pass"

    [late] = Members.invite(1, 1)
    Den.Repo.update_all(from(i in "invites", where: is_nil(i.used_at)), set: [expires_at: ~U[2020-01-01 00:00:00Z]])
    {_, pass3} = device_pass()
    assert Members.join(late, pass3) == {:error, :invite}, "an expired invite"
  end

  test "asking: nothing until the host approves; a declined request collects nothing", %{blinded: b} do
    {code, pass} = device_pass()
    assert Members.ask(pass, "") == {:error, :note}
    assert Members.ask(pass, String.duplicate("x", 281)) == {:error, :note}
    {:ok, id} = Members.ask(pass, "Hi, it's Ana from the reading group")
    assert Members.status(id) == {:ok, "pending"}
    assert Den.Door.sign(code, "2026-10-12", b) == {:error, :not_live}
    assert [%{note: "Hi, it's Ana from the reading group"}] = Members.pending()
    assert Members.approve(id) == :ok
    assert Members.status(id) == {:ok, "approved"}
    assert Members.pending() == []
    door_rules(code, b)
    assert Members.approve(id) == {:error, :request}, "decided once"

    {code2, pass2} = device_pass()
    {:ok, id2} = Members.ask(pass2, "let me in")
    assert Members.decline(id2) == :ok
    assert Members.approve(id2) == {:error, :request}, "a declined request cannot be approved after"
    assert Den.Door.sign(code2, "2026-10-12", b) == {:error, :not_live}
    assert Den.Repo.one(from(r in "requests", where: r.id == ^id2, select: r.note)) == nil, "the note is forgotten"
    assert Members.status("nope") == {:error, :request}
  end

  test "the host's site registers and removes passes with its token, and nobody else can", %{conn: conn, blinded: b} do
    {code, pass} = device_pass()
    refused = conn |> post("/door/passes", %{pass: pass}) |> json_response(401)
    assert refused["error"] =~ "host's own site"
    assert build_conn() |> put_req_header("authorization", "Bearer wrong") |> post("/door/passes", %{pass: pass}) |> json_response(401)
    assert Den.Door.sign(code, "2026-10-12", b) == {:error, :not_live}
    assert build_conn() |> put_req_header("authorization", "Bearer " <> @issuer) |> post("/door/passes", %{pass: pass}) |> json_response(200)
    door_rules(code, b)
    assert build_conn() |> put_req_header("authorization", "Bearer " <> @issuer) |> delete("/door/passes/" <> pass) |> json_response(200)
    assert Den.Door.sign(code, "2026-10-13", b) in [{:error, :not_live}, {:error, :no_key}]
    {code2, _} = device_pass()
    Application.put_env(:den, :today, fn -> ~D[2026-10-12] end)
    assert Den.Door.sign(code2, "2026-10-12", b) == {:error, :not_live}
  end

  test "a way the host has not opened is closed, and says so", %{conn: conn} do
    Application.put_env(:den, :entry, [:invite])
    {_, pass} = device_pass()
    assert Members.ask(pass, "hello") == {:error, :closed}
    assert Members.register(pass) == {:error, :closed}
    assert conn |> post("/door/ask", %{pass: pass, note: "hello"}) |> json_response(404)
    assert %{"invite" => true, "approval" => false} = build_conn() |> get("/door/ways") |> json_response(200)
    Application.put_env(:den, :entry, [:approval])
    [invite] = Members.invite(1, 7)
    assert Members.join(invite, pass) == {:error, :closed}
  end

  test "the endpoints: join, ask and its status, through HTTP", %{conn: conn} do
    [invite] = Members.invite(1, 7)
    {_, pass} = device_pass()
    assert conn |> post("/door/join", %{invite: invite, pass: pass}) |> json_response(200)
    assert build_conn() |> post("/door/join", %{invite: invite, pass: pass}) |> json_response(403)
    assert build_conn() |> post("/door/join", %{invite: invite, pass: "not hex"}) |> json_response(400)
    {_, pass2} = device_pass()
    %{"id" => id} = build_conn() |> post("/door/ask", %{pass: pass2, note: "hello"}) |> json_response(200)
    assert %{"status" => "pending"} = build_conn() |> get("/door/ask/" <> id) |> json_response(200)
  end

  test "a new den makes one first invite, and only while it has nobody and no invite" do
    code = Members.first_invite()
    assert code =~ ~r/\A[0-9a-f]{64}\z/
    assert Members.first_invite() == nil, "once there is an invite"
    {_, pass} = device_pass()
    assert Members.join(code, pass) == :ok
    assert Members.first_invite() == nil, "once there is a member"
  end

  test "the den keeps its own day keys: ahead, and none from before yesterday" do
    dir = Path.join(System.tmp_dir!(), "den-keep-#{System.unique_integer([:positive])}")
    was = Application.get_env(:den, :keys_dir)
    on_exit(fn -> Application.put_env(:den, :keys_dir, was) end)
    Application.put_env(:den, :keys_dir, dir)
    File.mkdir_p!(dir)
    for d <- ["2026-10-01", "2026-10-10"], do: File.write!(Path.join(dir, d <> ".pem"), "old")
    File.write!(Path.join(dir, "notes.txt"), "left alone")
    assert Den.Keys.keep(~D[2026-10-12], 3) == :ok
    assert File.ls!(dir) |> Enum.sort() == ["2026-10-12.pem", "2026-10-13.pem", "2026-10-14.pem", "notes.txt"]
    assert Den.Keys.days_ahead(~D[2026-10-12]) == 3
    assert {:ok, _} = Den.Keys.public_der("2026-10-13")
    before = File.read!(Path.join(dir, "2026-10-13.pem"))
    Den.Keys.keep(~D[2026-10-12], 3)
    assert File.read!(Path.join(dir, "2026-10-13.pem")) == before, "an existing key is never replaced"
    assert Den.Keys.keep(~D[2026-10-12], 0) == :ok
    File.rm_rf!(dir)
  end

  test "one source cannot fill the queue; the host can turn every waiting request down" do
    for _ <- 1..3, do: assert({:ok, _} = Members.ask(elem(device_pass(), 1), "hi", "src-a"))
    assert Members.ask(elem(device_pass(), 1), "hi", "src-a") == {:error, :busy}
    assert {:ok, _} = Members.ask(elem(device_pass(), 1), "hi", "src-b")
    assert Members.decline_all() == 4
    assert Members.pending() == []
    assert {:ok, _} = Members.ask(elem(device_pass(), 1), "hi again", "src-a"), "decided requests do not count"
    assert Repo.all(from r in "requests", where: not is_nil(r.decided), select: r.source) |> Enum.uniq() == [nil], "the source is forgotten with the note"
  end

  test "the door limits each address; behind a proxy on this machine, the forwarded one", %{conn: _} do
    Application.put_env(:den, :door_per_minute, 3)
    on_exit(fn -> Application.put_env(:den, :door_per_minute, 1_000_000) end)
    get = fn fwd -> build_conn() |> Map.put(:remote_ip, {127, 0, 0, 1}) |> put_req_header("x-forwarded-for", fwd) |> get("/door/ways") end
    for _ <- 1..3, do: assert(get.("198.51.100.1").status == 200)
    assert get.("198.51.100.1").status == 429
    assert get.("198.51.100.2").status == 200, "another address has its own budget"
    assert get.("203.0.113.9, 198.51.100.1").status == 429, "the proxy's own entry (the last) is the address"
    far = build_conn() |> Map.put(:remote_ip, {198, 51, 100, 77}) |> put_req_header("x-forwarded-for", "198.51.100.2")
    assert DenWeb.RateLimit.client(far) == {198, 51, 100, 77}, "a forwarded header from elsewhere is not believed"
    v6 = fn last -> build_conn() |> Map.put(:remote_ip, {0x2001, 0xDB8, 1, 2, 3, 4, 5, last}) |> get("/door/ways") end
    for i <- 1..3, do: assert(v6.(i).status == 200)
    assert v6.(99).status == 429, "an IPv6 /64 is one address: rotating inside it does not help"
    mapped = fn a -> build_conn() |> Map.put(:remote_ip, {0, 0, 0, 0, 0, 0xFFFF, a, 1}) end
    assert DenWeb.RateLimit.client(mapped.(0xC633)) == {198, 51, 0, 1}
    refute DenWeb.RateLimit.client(mapped.(0xC633)) == DenWeb.RateLimit.client(mapped.(0xCB00)), "IPv4 clients behind ::ffff: are not one bucket"
    before = :ets.info(DenWeb.RateLimit, :size)
    assert before > 0
    :ets.insert(DenWeb.RateLimit, {{{:door, {9, 9, 9, 9}}, 0}, 5})
    DenWeb.RateLimit.sweep()
    assert :ets.lookup(DenWeb.RateLimit, {{:door, {9, 9, 9, 9}}, 0}) == [], "ended windows are swept"
  end

  test "the host's commands print what they say, with real dates" do
    import ExUnit.CaptureIO
    [_] = Members.invite(1, 3)
    {:ok, _} = Members.ask(elem(device_pass(), 1), "hello there")
    assert capture_io(fn -> Den.Host.invites() end) =~ ~r/made 20\d\d-.* open until 20\d\d-/
    assert capture_io(fn -> Den.Host.requests() end) =~ "hello there"
    assert capture_io(fn -> Den.Host.invite(2, 1) end) |> String.split("\n", trim: true) |> length() == 2
    assert capture_io(fn -> Den.Host.invite(420, 1) end) =~ "1 to 100 at a time"
    assert %{} = Den.Host.members()
  end

  test "the host names the den's owner by the start of its device's name" do
    import ExUnit.CaptureIO
    a = "abc123" <> String.duplicate("0", 58)
    b = "abc1239" <> String.duplicate("0", 57)
    later = DateTime.add(DateTime.utc_now(), 3600, :second) |> DateTime.truncate(:second)
    Den.Rooms.admit(a, later)
    Den.Rooms.admit(b, later)
    assert capture_io(fn -> Den.Host.owner() end) =~ "no owner"
    assert capture_io(fn -> Den.Host.owner("m-abc123") end) =~ "Several devices"
    assert capture_io(fn -> Den.Host.owner("m-ab") end) =~ "at least six"
    assert capture_io(fn -> Den.Host.owner("m-ffffff") end) =~ "No device in the den today"
    assert capture_io(fn -> Den.Host.owner("m-abc1230") end) =~ "owns the den now"
    assert Den.Mod.owner() == a
    :ok = Den.Mod.timeout(a, b, 600, "test")
    assert capture_io(fn -> Den.Host.audit() end) =~ ~r/m-abc1230000  timeout 600s m-abc1239000  \(test\)/
    assert capture_io(fn -> Den.Host.owner(:clear) end) =~ "no owner now"
  end

  test "a pass hash is SHA-256 over a zero byte, the domain and the code, as the page makes it" do
    assert Base.encode16(Members.hash(:binary.copy(<<0x11>>, 32)), case: :lower) == "89fbbb8554656bd7bdd0f200b552b67d6cc3c8faabd86261531e1d49ce822833"
  end

  test "no invite, pass or note is logged", %{conn: conn} do
    _ = conn
    keys = ~w(invite pass note code sig blinded msg randomizer proof session)
    shown = Phoenix.Logger.filter_values(Map.new(keys, &{&1, "secret"}))
    for k <- keys, do: assert(shown[k] == "[FILTERED]", k)
  end
end
