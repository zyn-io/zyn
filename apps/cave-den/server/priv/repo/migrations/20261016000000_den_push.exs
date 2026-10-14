defmodule Den.Repo.Migrations.DenPush do
  use Ecto.Migration

  # Push addresses (Den.Push): a device's browser push endpoint, its keys for
  # encrypting to it, and which rooms it wants pushes for.
  def change do
    create table(:push_subs, primary_key: false) do
      add :endpoint, :string, primary_key: true
      add :key, :string, null: false
      add :p256dh, :string, null: false
      add :auth, :string, null: false
      add :mode, :string, null: false, default: "private"
      add :made_at, :utc_datetime, null: false
    end

    create index(:push_subs, [:key])
  end
end
