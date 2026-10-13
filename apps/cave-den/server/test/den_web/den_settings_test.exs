defmodule DenWeb.DenSettingsTest do
  use DenWeb.ConnCase, async: false

  test "the door says the den's name and serves its icon", %{conn: conn} do
    assert %{"name" => "", "icon" => false} = conn |> get("/door/ways") |> json_response(200)
    assert build_conn() |> get("/door/icon") |> response(404)
    owner = String.duplicate("0a", 32)
    :ok = Den.Mod.set_owner(owner)
    png = <<0x89, "PNG", 13, 10, 26, 10>>
    :ok = Den.Mod.settings_save(owner, %{"name" => "Zyn community", "icon" => "data:image/png;base64," <> Base.encode64(png)})
    assert %{"name" => "Zyn community", "icon" => true} = build_conn() |> get("/door/ways") |> json_response(200)
    r = build_conn() |> get("/door/icon")
    assert r.resp_body == png
    assert ["image/png"] = get_resp_header(r, "content-type")
  end
end
