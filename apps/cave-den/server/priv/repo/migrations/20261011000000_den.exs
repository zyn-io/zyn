defmodule Den.Repo.Migrations.Den do
  use Ecto.Migration

  def change do
    # A room: an MLS group the den orders but cannot read.
    create table(:rooms, primary_key: false) do
      add :id, :string, primary_key: true
      add :title, :string, null: false
      add :epoch, :integer, null: false, default: 0
      add :group_info, :binary
      add :seq, :integer, null: false, default: 0
      timestamps(type: :utc_datetime)
    end

    # Ciphertext, in the order the den relayed it.
    create table(:messages) do
      add :room, :string, null: false
      add :seq, :integer, null: false
      add :epoch, :integer, null: false
      add :commit, :boolean, null: false
      add :bytes, :binary, null: false
      timestamps(type: :utc_datetime, updated_at: false)
    end

    create unique_index(:messages, [:room, :seq])

    # A device admitted with a day token: its key and until when.
    create table(:devices, primary_key: false) do
      add :key, :string, primary_key: true
      add :until, :utc_datetime, null: false
      timestamps(type: :utc_datetime)
    end

    # Which device keys joined which room: what lets members take out a
    # device whose day ended.
    create table(:room_devices, primary_key: false) do
      add :room, :string, primary_key: true
      add :key, :string, primary_key: true
      timestamps(type: :utc_datetime)
    end

    # Tokens the door signed per pass per day — the only place a pass is
    # kept, and only for two days.
    create table(:issued, primary_key: false) do
      add :pass, :string, primary_key: true
      add :day, :string, primary_key: true
      add :count, :integer, null: false
    end

    # Tokens already used.
    create table(:spent, primary_key: false) do
      add :token, :string, primary_key: true
      add :day, :string, null: false
    end
  end
end
