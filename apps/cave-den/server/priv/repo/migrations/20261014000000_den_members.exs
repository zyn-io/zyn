defmodule Den.Repo.Migrations.DenMembers do
  use Ecto.Migration

  # The den's own member list (Den.Gate.Members): pass hashes, the invites
  # that let them in, and requests waiting for the host. Nothing that exists
  # changes.
  def change do
    create table(:members, primary_key: false) do
      add :pass, :text, primary_key: true
      add :how, :text, null: false
      add :ref, :text
      add :added_at, :utc_datetime, null: false
    end

    create index(:members, [:how, :ref])

    create table(:invites, primary_key: false) do
      add :id, :text, primary_key: true
      add :made_at, :utc_datetime, null: false
      add :expires_at, :utc_datetime, null: false
      add :used_at, :utc_datetime
      add :revoked_at, :utc_datetime
    end

    create table(:requests, primary_key: false) do
      add :id, :text, primary_key: true
      add :pass, :text, null: false
      add :note, :text
      add :asked_at, :utc_datetime, null: false
      add :decided, :text
    end
  end
end
