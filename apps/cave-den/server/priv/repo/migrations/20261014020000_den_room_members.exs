defmodule Den.Repo.Migrations.DenRoomMembers do
  use Ecto.Migration

  # Each room's member keys, as its public state names them after the last
  # commit (the den checks writers and private-room joins against them), and
  # who made a private room (the only one who may enter it before its group
  # exists). Rooms made before this are filled in on their next use.
  def change do
    alter table(:rooms) do
      add :members, :text
      add :creator, :text
    end
  end
end
