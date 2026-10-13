defmodule Den.Repo.Migrations.DenSweepIndexes do
  use Ecto.Migration

  # What the sweeper reads every minute: by time, not by whole tables.
  def change do
    create_if_not_exists index(:messages, [:inserted_at])
    create_if_not_exists index(:devices, [:until])
    create_if_not_exists index(:room_devices, [:key])
    create_if_not_exists index(:issued, [:day])
    create_if_not_exists index(:spent, [:day])
    create_if_not_exists index(:quota, [:day])
    create_if_not_exists index(:inbox, [:inserted_at])
    create_if_not_exists index(:blobs, [:inserted_at])
    create_if_not_exists index(:barred, [:until])
  end
end
