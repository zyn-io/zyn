defmodule Den.Repo.Migrations.DenPublicState do
  use Ecto.Migration

  # Each room keeps its public MLS state (Den.Public). Rooms made before the
  # den checked commits start again: their groups are not tracked.
  def change do
    alter table(:rooms) do
      add :public_state, :text
    end

    execute "UPDATE rooms SET group_info = NULL, epoch = 0", ""
    execute "DELETE FROM messages", ""
  end
end
