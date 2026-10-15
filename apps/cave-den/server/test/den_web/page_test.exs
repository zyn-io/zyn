defmodule DenWeb.PageTest do
  use DenWeb.ConnCase, async: false

  # A stand-in page, unless a real one has been built into priv/page.
  setup do
    dir = Application.app_dir(:den, "priv/page")
    made = not File.exists?(Path.join(dir, "index.html"))

    if made do
      File.mkdir_p!(dir)
      File.write!(Path.join(dir, "index.html"), "<!doctype html><title>Den</title>")
      File.write!(Path.join(dir, "app.js"), "export {};")
      File.write!(Path.join(dir, "config.json"), ~s({"keysUrl": "/door/keys", "prefix": "den", "issuer": null}))
    end

    on_exit(fn -> if made, do: File.rm_rf!(dir) end)
    :ok
  end

  test "the den serves its own page under a policy with no extension or inline script", %{conn: conn} do
    conn = get(conn, "/")
    assert conn.status == 200
    assert conn.resp_body =~ "<title>"
    [csp] = get_resp_header(conn, "content-security-policy")
    assert csp =~ "script-src 'self' 'wasm-unsafe-eval';"
    refute csp =~ ~r/extension:|unsafe-inline|unsafe-eval'[^;]*unsafe/
    assert csp =~ "frame-ancestors 'none'"
    assert get_resp_header(conn, "permissions-policy") == ["geolocation=(), microphone=(self), camera=(), payment=()"]
    assert get_resp_header(conn, "cross-origin-opener-policy") == ["same-origin-allow-popups"]
  end

  test "the page's files carry the same policy; the door's answers are left alone", %{conn: conn} do
    page = get(conn, "/app.js")
    assert page.status == 200
    assert [_] = get_resp_header(page, "content-security-policy")
    door = get(build_conn(), "/door/keys")
    assert get_resp_header(door, "content-security-policy") == []
  end

  test "config.json names the issuer the host set, over the built file's keys", %{conn: conn} do
    plain = get(conn, "/config.json")
    assert plain.status == 200
    refute Map.has_key?(Jason.decode!(plain.resp_body), "issuerLabel")

    Application.put_env(:den, :issuer, "https://access.example.org/")
    Application.put_env(:den, :issuer_label, "Get access")

    on_exit(fn ->
      Application.delete_env(:den, :issuer)
      Application.delete_env(:den, :issuer_label)
    end)

    conn = get(build_conn(), "/config.json")
    assert conn.status == 200
    assert [_] = get_resp_header(conn, "content-security-policy")
    config = Jason.decode!(conn.resp_body)
    assert config["issuer"] == "https://access.example.org/"
    assert config["issuerLabel"] == "Get access"
    assert Map.has_key?(config, "prefix")
  end

  test "the policy names the den's socket and the extra origins" do
    assert DenWeb.Page.csp("chat.example.org", ["https://keys.example.org"]) =~
             "connect-src 'self' wss://chat.example.org https://keys.example.org;"
  end

  test "a socket from the den's own page is allowed; another site's only if listed" do
    assert DenWeb.Origins.allowed?(URI.parse(DenWeb.Endpoint.url()))
    refute DenWeb.Origins.allowed?(URI.parse("https://elsewhere.example"))
  end
end
