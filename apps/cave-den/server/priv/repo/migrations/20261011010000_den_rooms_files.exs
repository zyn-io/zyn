defmodule Den.Repo.Migrations.DenRoomsFiles do
  use Ecto.Migration

  def change do
    # open: any member joins; private: by invitation, never by GroupInfo;
    # proposal: open, made by the den for a ZCaveDAO proposal.
    alter table(:rooms) do
      add :kind, :string, null: false, default: "open"
    end

    # One-time invitations to a device, handed out once each.
    create table(:key_packages) do
      add :key, :string, null: false
      add :bytes, :binary, null: false
      timestamps(type: :utc_datetime, updated_at: false)
    end

    create index(:key_packages, [:key])

    # Welcomes waiting for a device.
    create table(:inbox) do
      add :key, :string, null: false
      add :room, :string, null: false
      add :bytes, :binary, null: false
      timestamps(type: :utc_datetime, updated_at: false)
    end

    create index(:inbox, [:key])

    # Encrypted files: the den keeps ciphertext under its hash, for a while.
    create table(:blobs, primary_key: false) do
      add :id, :string, primary_key: true
      add :size, :integer, null: false
      timestamps(type: :utc_datetime, updated_at: false)
    end

    # What a device made today: rooms, invitations claimed, files.
    create table(:quota, primary_key: false) do
      add :key, :string, primary_key: true
      add :day, :string, primary_key: true
      add :what, :string, primary_key: true
      add :count, :integer, null: false
    end
  end
end
