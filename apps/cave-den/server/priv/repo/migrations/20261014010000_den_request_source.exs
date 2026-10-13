defmodule Den.Repo.Migrations.DenRequestSource do
  use Ecto.Migration

  # A keyed tag of the address a request came from, while it waits: at most a
  # few waiting from one source (Den.Gate.Members.ask/3).
  def change do
    alter table(:requests) do
      add :source, :text
    end

    create index(:requests, [:decided, :source])
  end
end
