defmodule Den.ModTest do
  use Den.DataCase, async: false
  alias Den.Mod

  @owner String.duplicate("0a", 32)
  @mod String.duplicate("1b", 32)
  @helper String.duplicate("2c", 32)
  @member String.duplicate("3d", 32)

  setup do
    Den.Rooms.seed()
    :ok = Mod.set_owner(@owner)
    {:ok, mod} = Mod.role_save(@owner, %{"name" => "Moderator", "colour" => "#E0A030", "hoist" => true, "position" => 10, "perms" => ["timeout", "kick", "manage_messages", "manage_roles"]})
    {:ok, helper} = Mod.role_save(@owner, %{"name" => "Helper", "position" => 5, "perms" => ["timeout"]})
    :ok = Mod.role_give(@owner, @mod, mod.id)
    :ok = Mod.role_give(@owner, @helper, helper.id)
    %{mod: mod, helper: helper}
  end

  test "the owner holds every permission and is above everyone; nobody can act on the owner" do
    assert Mod.governed?()
    assert Mod.perms(@owner) == MapSet.new(Mod.perm_names())
    assert Mod.above?(@owner, @mod)
    refute Mod.above?(@mod, @owner)
    assert Mod.timeout(@mod, @owner, 600) == {:error, :above}
    assert Mod.ban(@owner, @owner) == {:error, :self}
  end

  test "permissions are the union of a device's roles and @everyone's; Administrator is all", %{helper: helper} do
    assert %{hoist: true, colour: "#e0a030"} = Mod.role(hd(Mod.roles_of(@mod)))
    assert Mod.can?(@mod, "kick")
    refute Mod.can?(@mod, "ban")
    refute Mod.can?(@member, "timeout")
    {:ok, _} = Mod.role_save(@owner, %{"id" => "everyone", "perms" => ["bypass_slow"]})
    assert Mod.can?(@member, "bypass_slow")
    assert Mod.can?(@helper, "bypass_slow") and Mod.can?(@helper, "timeout")
    {:ok, _} = Mod.role_save(@owner, %{"id" => helper.id, "name" => "Helper", "position" => 5, "perms" => ["administrator"]})
    assert Mod.can?(@helper, "ban")
  end

  test "the order of roles: a moderator acts only below its own highest role", %{helper: helper} do
    assert Mod.timeout(@mod, @helper, 600, "flooding") == :ok
    assert Mod.muted?(@helper)
    assert Mod.timeout(@helper, @mod, 600) == {:error, :forbidden} or Mod.timeout(@helper, @mod, 600) == {:error, :above}
    # A helper (timeout) cannot time out the moderator above it.
    :ok = Mod.untimeout(@mod, @helper)
    assert Mod.timeout(@helper, @mod, 600) == {:error, :above}
    assert Mod.timeout(@helper, @member, 600) == :ok
    # Equal roles cannot act on each other.
    other = String.duplicate("4e", 32)
    :ok = Mod.role_give(@owner, other, helper.id)
    assert Mod.timeout(@helper, other, 600) == {:error, :above}
  end

  test "roles are made, edited and given only below one's own, and only with permissions one holds", %{mod: mod, helper: helper} do
    # The moderator (position 10) may make a role below it, with its own permissions only.
    assert {:ok, r} = Mod.role_save(@mod, %{"name" => "Trial", "position" => 3, "perms" => ["timeout"]})
    assert Mod.role_save(@mod, %{"name" => "Big", "position" => 3, "perms" => ["ban"]}) == {:error, :perm}
    assert Mod.role_save(@mod, %{"name" => "Equal", "position" => 10, "perms" => []}) == {:error, :above}
    assert Mod.role_save(@mod, %{"id" => mod.id, "name" => "Moderator", "position" => 10, "perms" => ["timeout"]}) == {:error, :above}
    assert Mod.role_give(@mod, @member, r.id) == :ok
    assert Mod.role_give(@mod, @member, mod.id) == {:error, :above}
    assert Mod.role_take(@mod, @member, r.id) == :ok
    refute r.id in Mod.roles_of(@member)
    # Giving a role to someone at or above oneself: no.
    assert Mod.role_give(@mod, @owner, helper.id) == {:error, :above}
    assert Mod.role_delete(@mod, mod.id) == {:error, :above}
    assert Mod.role_delete(@mod, r.id) == :ok
    assert Mod.role_delete(@owner, "everyone") == {:error, :role}
    assert Mod.role_save(@member, %{"name" => "Me", "perms" => []}) == {:error, :forbidden}
    assert Mod.role_save(@owner, %{"name" => "", "perms" => []}) == {:error, :name}
    assert Mod.role_save(@owner, %{"name" => "x", "colour" => "red", "perms" => []}) == {:error, :colour}
    assert Mod.role_save(@owner, %{"name" => "x", "perms" => ["root"]}) == {:error, :perm}
  end

  test "timeouts end by themselves; kicks and bans; a weaker sanction never replaces a stronger one" do
    assert Mod.timeout(@mod, @member, 30) == {:error, :duration}
    assert Mod.timeout(@mod, @member, 60) == :ok
    assert %{kind: "timeout"} = Mod.sanction(@member)
    Den.Repo.update_all(Ecto.Query.from(s in "sanctions"), set: [until: DateTime.add(DateTime.utc_now(), -1, :second) |> DateTime.truncate(:second)])
    refute Mod.muted?(@member)
    assert Mod.kick(@mod, @member, "spam") == :ok
    assert Mod.out?(@member) and not Mod.banned?(@member)
    assert Mod.ban(@mod, @member) == {:error, :forbidden}
    assert Mod.ban(@owner, @member, "spam bot") == :ok
    assert Mod.timeout(@mod, @member, 600) == :ok
    assert Mod.banned?(@member), "the ban stays"
    assert [%{key: @member, reason: "spam bot"}] = Mod.bans()
    assert Mod.unban(@owner, @member) == :ok
    refute Mod.out?(@member)
  end

  test "the audit log says who did what to whom, never what anyone wrote" do
    :ok = Mod.timeout(@mod, @member, 600, "flooding")
    assert [%{actor: @mod, action: "timeout 600s", target: @member, reason: "flooding"} | _] = Mod.audit(5)
    assert Mod.moderator?(@helper)
    refute Mod.moderator?(@member)
  end

  test "ownership passes only from the owner; the host can clear it" do
    assert Mod.owner_pass(@mod, @mod) == {:error, :forbidden}
    assert Mod.owner_pass(@owner, @mod) == :ok
    assert Mod.owner?(@mod)
    :ok = Mod.set_owner(nil)
    refute Mod.governed?()
    assert Mod.perms(@mod) == MapSet.new(["timeout", "kick", "manage_messages", "manage_roles"])
  end

  test "settings: name, description, an icon that is an image, limits under the host's" do
    png = "data:image/png;base64," <> Base.encode64(<<0x89, "PNG", 0, 1, 2>>)
    assert Mod.settings_save(@mod, %{"name" => "Zyn"}) == {:error, :forbidden}
    assert Mod.settings_save(@owner, %{"name" => "Zyn community", "description" => "Builders", "icon" => png}) == :ok
    assert %{den: %{name: "Zyn community", icon: true}} = Mod.public()
    assert {"image/png", _} = Mod.icon_bytes()
    assert Mod.settings_save(@owner, %{"icon" => "data:image/svg+xml;base64,PHN2Zy8+"}) == {:error, :icon}
    assert Mod.settings_save(@owner, %{"icon" => "data:image/png;base64," <> Base.encode64("not a png")}) == {:error, :icon}
    assert Mod.settings_save(@owner, %{"limits" => %{"file_max" => 10_000_000}}) == :ok
    assert Den.Limits.get(:file_max) == 10_000_000
    assert Mod.settings_save(@owner, %{"limits" => %{"file_max" => 10 * Den.Limits.host(:file_max)}}) == :ok
    assert Den.Limits.get(:file_max) == Den.Limits.host(:file_max), "never above the host's"
    assert Mod.settings_save(@owner, %{"limits" => %{"room_members" => 5}}) == {:error, :limits}
  end

  test "an open room's settings, by Manage channels; a renamed host room keeps its name", %{mod: mod} do
    assert Mod.room_save(@mod, "general", %{"slow" => 30}) == {:error, :forbidden}
    {:ok, _} = Mod.role_save(@owner, %{"id" => mod.id, "name" => "Moderator", "position" => 10, "perms" => ["timeout", "kick", "manage_messages", "manage_roles", "manage_channels"]})
    assert {:ok, r} = Mod.room_save(@mod, "general", %{"slow" => 30, "topic" => "Say hi", "title" => "Lobby", "post_roles" => [mod.id]})
    assert %{slow_seconds: 30, topic: "Say hi", title: "Lobby"} = r
    assert Mod.room_save(@mod, "general", %{"slow" => 31}) == {:error, :slow}
    assert Mod.may_post?(r, @mod)
    refute Mod.may_post?(r, @member)
    assert Mod.may_post?(r, @owner)
    Den.Rooms.seed()
    assert Den.Rooms.get("general").title == "Lobby"
  end
end
