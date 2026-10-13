defmodule Den.Repo.Migrations.DenCommitReports do
  use Ecto.Migration

  # A room keeps the state from before its last commit, so that commit can be
  # undone when most members report they cannot apply it (Den.Rooms.report/5);
  # the reports; and senders barred from committing in a room for a day.
  def change do
    alter table(:rooms) do
      add :prev_state, :text
      add :prev_members, :text
      add :prev_group_info, :binary
      add :last_commit_seq, :integer
      add :last_committer, :text
      add :last_commit_at, :utc_datetime
      # The epoch of the GroupInfo kept for joiners.
      add :info_epoch, :integer
    end

    create table(:reports, primary_key: false) do
      add :room, :string, null: false
      add :seq, :integer, null: false
      add :key, :string, null: false
      add :inserted_at, :utc_datetime, null: false
    end

    create unique_index(:reports, [:room, :seq, :key])

    create table(:barred, primary_key: false) do
      add :room, :string, null: false
      add :key, :string, null: false
      add :until, :utc_datetime, null: false
    end

    create unique_index(:barred, [:room, :key])
  end
end
