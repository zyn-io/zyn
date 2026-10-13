defmodule Den.Repo.Migrations.DenMod do
  use Ecto.Migration

  # Roles and moderation (Den.Mod): roles with permissions, who holds them,
  # timeouts, kicks and bans by device key, what moderators did, the den's own
  # settings (its owner, name, icon, limits), and each open room's settings.
  def change do
    create table(:roles, primary_key: false) do
      add :id, :string, primary_key: true
      add :name, :string, null: false
      add :colour, :string
      add :position, :integer, null: false, default: 1
      add :hoist, :boolean, null: false, default: false
      add :perms, :text, null: false, default: ""
      add :inserted_at, :utc_datetime, null: false
    end

    create table(:role_keys, primary_key: false) do
      add :role, :string, null: false
      add :key, :string, null: false
    end

    create unique_index(:role_keys, [:role, :key])
    create index(:role_keys, [:key])

    create table(:sanctions, primary_key: false) do
      add :key, :string, primary_key: true
      add :kind, :string, null: false
      add :until, :utc_datetime
      add :by, :string
      add :reason, :text
      add :at, :utc_datetime, null: false
    end

    create table(:audit) do
      add :at, :utc_datetime, null: false
      add :actor, :string
      add :action, :string, null: false
      add :target, :string
      add :reason, :text
    end

    create index(:audit, [:at])

    create table(:settings, primary_key: false) do
      add :key, :string, primary_key: true
      add :value, :text
    end

    alter table(:rooms) do
      add :slow_seconds, :integer, null: false, default: 0
      add :post_roles, :text
      add :topic, :text
      add :position, :integer
    end
  end
end
