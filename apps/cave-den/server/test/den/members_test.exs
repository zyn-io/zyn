defmodule Den.MembersTest do
  use DenWeb.ConnCase, async: false
  import Ecto.Query
  alias Den.Gate.Members

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
    blinded = String.trim(File.read!(Path.join(@f, "blinded.hex")))
    on_exit(fn ->
      Application.delete_env(:den, :today)
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
