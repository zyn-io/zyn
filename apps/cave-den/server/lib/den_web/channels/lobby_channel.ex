defmodule DenWeb.LobbyChannel do
  @moduledoc """
  What every admitted device shares: the list of open and proposal rooms, new
  rooms as they are made, and the means to start a private one or invite a
  device to it.

  * join → the rooms.
  * "open_room" {title} → a room any member can join (3 a day per device).
  * "private_room" {id} → reserve an unguessable private room id.
  * "claim" {key} → one of that device's invitations (KeyPackages).

  Moderation (Den.Mod), each checked against the sender's permissions and
  the order of roles: "role_save", "role_delete", "role_give", "role_take",
  "timeout", "untimeout", "kick", "ban", "unban", "bans", "room_save",
  "room_delete", "settings_save", "audit", "invite_make", "invites",
  "invite_withdraw", "requests", "approve", "decline", "owner_pass". After a
  change everyone is sent "mod" (roles, holders, sanctions, the den's name),
  "room" or "room_gone".
  """
  # Joins and messages are not logged: a topic names a device or a private room.
  use Phoenix.Channel, log_join: false, log_handle_in: false
  alias Den.Rooms

  @impl true
  def join("lobby", _, socket) do
    DenWeb.Heap.channel()
    {:ok, %{rooms: Rooms.listed(), mod: Den.Mod.for_device(socket.assigns.key)}, socket}
  end

  # Every event, at a rate the den can answer (DenWeb.Throttle).
  @impl true
  def handle_in(event, payload, socket) do
    cond do
      DenWeb.Throttle.allow?(event) -> handle(event, payload, socket)
      event == "typing" -> {:noreply, socket}
      true -> {:reply, DenWeb.Throttle.refused(), socket}
    end
  end

  defp handle("open_room", %{"title" => title}, socket) when is_binary(title) do
    # A den with an owner: channels are made by Manage channels, as many as
    # they like; without one, any device makes 3 a day.
    allowed =
      if Den.Mod.governed?(),
        do: if(Den.Mod.can?(socket.assigns.key, "manage_channels"), do: :ok, else: {:error, :forbidden}),
        else: Rooms.quota(socket.assigns.key, "room", 3)

    with :ok <- allowed,
         {:ok, r} <- Rooms.open_room(title) do
      if Den.Mod.governed?(), do: Den.Mod.log(socket.assigns.key, "channel made", r.id, r.title)
      broadcast!(socket, "room", Rooms.public(r))
      {:reply, {:ok, %{id: r.id, title: r.title}}, socket}
    else
      {:error, why} -> {:reply, {:error, %{reason: said(why, "room")}}, socket}
    end
  end

  defp handle("private_room", %{"id" => id}, socket) when is_binary(id) do
    with :ok <- Rooms.quota(socket.assigns.key, "private", 50),
         {:ok, r} <- Rooms.private_room(id, socket.assigns.key) do
      {:reply, {:ok, %{id: r.id}}, socket}
    else
      {:error, why} -> {:reply, {:error, %{reason: said(why, "private")}}, socket}
    end
  end

  defp handle("claim", %{"key" => key}, socket) when is_binary(key) do
    # Invitations are the target device's: one member may take few of them,
    # and all members together a few more, each day.
    with true <- key =~ ~r/\A[0-9a-f]{64}\z/ || {:error, :key},
         :ok <- Rooms.quota(socket.assigns.key, "claim", 100),
         :ok <- Rooms.quota(socket.assigns.key, "claim:" <> key, 3),
         :ok <- Rooms.quota(key, "claimed", 20),
         {:ok, bytes} <- Den.Inbox.claim(key) do
      {:reply, {:ok, %{key_package: Base.encode64(bytes)}}, socket}
    else
      {:error, :none} -> {:reply, {:error, %{reason: "That member's device has to come online once before it can be invited."}}, socket}
      {:error, why} -> {:reply, {:error, %{reason: said(why, "claim")}}, socket}
    end
  end

  # ---- Moderation -------------------------------------------------------------

  defp handle("role_save", %{} = attrs, socket), do: changed(Den.Mod.role_save(me(socket), attrs), socket)
  defp handle("role_delete", %{"id" => id}, socket), do: changed(Den.Mod.role_delete(me(socket), id), socket)
  defp handle("role_give", %{"key" => k, "id" => id}, socket), do: changed(Den.Mod.role_give(me(socket), k, id), socket)
  defp handle("role_take", %{"key" => k, "id" => id}, socket), do: changed(Den.Mod.role_take(me(socket), k, id), socket)
  defp handle("timeout", %{"key" => k, "seconds" => s} = p, socket), do: changed(Den.Mod.timeout(me(socket), k, s, p["reason"]), socket)
  defp handle("untimeout", %{"key" => k}, socket), do: changed(Den.Mod.untimeout(me(socket), k), socket)
  defp handle("kick", %{"key" => k} = p, socket), do: changed(Den.Mod.kick(me(socket), k, p["reason"]), socket, fn -> Rooms.expel(k) end)
  defp handle("ban", %{"key" => k} = p, socket), do: changed(Den.Mod.ban(me(socket), k, p["reason"]), socket, fn -> Rooms.expel(k) end)
  defp handle("unban", %{"key" => k}, socket), do: changed(Den.Mod.unban(me(socket), k), socket)
  defp handle("settings_save", %{} = attrs, socket), do: changed(Den.Mod.settings_save(me(socket), attrs), socket)
  defp handle("owner_pass", %{"key" => k}, socket), do: changed(Den.Mod.owner_pass(me(socket), k), socket)

  defp handle("bans", _, socket) do
    if Den.Mod.can?(me(socket), "ban"),
      do: {:reply, {:ok, %{bans: for(b <- Den.Mod.bans(), do: %{b | at: DateTime.to_unix(b.at)})}}, socket},
      else: no(:forbidden, socket)
  end

  defp handle("audit", p, socket) do
    n = if is_integer(p["n"]) and p["n"] in 1..500, do: p["n"], else: 100

    if Den.Mod.moderator?(me(socket)),
      do: {:reply, {:ok, %{entries: for(a <- Den.Mod.audit(n), do: %{a | at: DateTime.to_unix(a.at)})}}, socket},
      else: no(:forbidden, socket)
  end

  defp handle("room_save", %{"id" => id} = p, socket) when is_binary(id) do
    case Den.Mod.room_save(me(socket), id, Map.delete(p, "id")) do
      {:ok, r} ->
        broadcast!(socket, "room", Rooms.public(r))
        {:reply, {:ok, Rooms.public(r)}, socket}

      {:error, why} ->
        no(why, socket)
    end
  end

  defp handle("room_delete", %{"id" => id}, socket) when is_binary(id) do
    with true <- Den.Mod.can?(me(socket), "manage_channels") || {:error, :forbidden},
         :ok <- Rooms.delete_room(id) do
      Den.Mod.log(me(socket), "channel deleted", id, nil)
      broadcast!(socket, "room_gone", %{id: id})
      {:reply, :ok, socket}
    else
      {:error, why} -> no(why, socket)
    end
  end

  # Invites and requests to join, for Manage invites (the host's commands, in the page).
  defp handle("invite_make", p, socket) do
    n = p["n"] || 1
    days = p["days"] || 7

    cond do
      not Den.Mod.can?(me(socket), "manage_invites") -> no(:forbidden, socket)
      not (Den.Gate.module() == Den.Gate.Members and Den.Gate.Members.open?(:invite)) -> no(:no_invites, socket)
      not (is_integer(n) and n in 1..20 and is_integer(days) and days in 1..90) -> no(:invite_count, socket)
      true ->
        base = DenWeb.Endpoint.url()
        links = for code <- Den.Gate.Members.invite(n, days), do: "#{base}/#invite=#{code}"
        Den.Mod.log(me(socket), "invites made", nil, "#{n} for #{days} days")
        {:reply, {:ok, %{links: links}}, socket}
    end
  end

  defp handle("invites", _, socket) do
    if Den.Mod.can?(me(socket), "manage_invites") and Den.Gate.module() == Den.Gate.Members do
      list =
        for i <- Den.Gate.Members.invites() do
          %{id: String.slice(i.id, 0, 12), made_at: unix(i.made_at), expires_at: unix(i.expires_at), used_at: unix(i.used_at), revoked_at: unix(i.revoked_at)}
        end

      {:reply, {:ok, %{invites: list}}, socket}
    else
      no(:forbidden, socket)
    end
  end

  defp handle("invite_withdraw", %{"id" => id}, socket) when is_binary(id) do
    with true <- Den.Mod.can?(me(socket), "manage_invites") || {:error, :forbidden},
         {:ok, _} <- Den.Gate.Members.withdraw(id) do
      Den.Mod.log(me(socket), "invite withdrawn", id, nil)
      {:reply, :ok, socket}
    else
      {:error, why} -> no(why, socket)
    end
  end

  defp handle("requests", _, socket) do
    if Den.Mod.can?(me(socket), "manage_invites") and Den.Gate.module() == Den.Gate.Members,
      do: {:reply, {:ok, %{requests: for(r <- Den.Gate.Members.pending(), do: %{r | asked_at: unix(r.asked_at)})}}, socket},
      else: no(:forbidden, socket)
  end

  defp handle(e, %{"id" => id}, socket) when e in ["approve", "decline"] and is_binary(id) do
    with true <- Den.Mod.can?(me(socket), "manage_invites") || {:error, :forbidden},
         :ok <- apply(Den.Gate.Members, String.to_existing_atom(e), [id]) do
      Den.Mod.log(me(socket), "request #{e}d", nil, nil)
      {:reply, :ok, socket}
    else
      {:error, why} -> no(why, socket)
      other -> no(other, socket)
    end
  end

  defp handle(_, _, socket), do: {:reply, {:error, %{reason: "unknown"}}, socket}

  defp me(socket), do: socket.assigns.key
  defp unix(nil), do: nil
  defp unix(t), do: DateTime.to_unix(t)

  # After a change: everyone's pages hear the den's moderation state again.
  defp changed(result, socket, after_ok \\ fn -> :ok end) do
    case result do
      ok when ok == :ok or elem(ok, 0) == :ok ->
        after_ok.()
        broadcast!(socket, "mod", Den.Mod.public())
        {:reply, :ok, socket}

      {:error, why} ->
        no(why, socket)
    end
  end

  defp no(why, socket), do: {:reply, {:error, %{reason: said(why, "mod")}}, socket}

  # What a member reads when the den says no.
  defp said(:quota, "room"), do: "A device can open 3 rooms a day. Try again tomorrow."
  defp said(:quota, "private"), do: "A device can start 50 private rooms a day. Try again tomorrow."
  defp said(:quota, "claim"), do: "That is enough invitations for today. Try again tomorrow."
  defp said(:title, "room"), do: "Give the room a name."
  defp said(:forbidden, _), do: "You do not have permission to do that."
  defp said(:above, _), do: "You can only do that to members and roles below your highest role."
  defp said(:self, _), do: "You cannot do that to yourself."
  defp said(:key, _), do: "That is not a member's device."
  defp said(:role, _), do: "There is no such role."
  defp said(:perm, _), do: "You cannot give a permission you do not have."
  defp said(:name, _), do: "Give it a name of up to 32 characters (the den's name: up to 40)."
  defp said(:colour, _), do: "A colour is written #rrggbb."
  defp said(:position, _), do: "That is not a place in the list of roles."
  defp said(:limit, _), do: "A den has at most 50 roles."
  defp said(:duration, _), do: "A timeout lasts from a minute to 28 days."
  defp said(:reason, _), do: "Keep the reason under 300 characters."
  defp said(:slow, _), do: "Pick one of the slow mode choices."
  defp said(:title, _), do: "Give the channel a name of up to 40 characters."
  defp said(:topic, _), do: "Keep the topic under 200 characters."
  defp said(:description, _), do: "Keep the description under 300 characters."
  defp said(:icon, _), do: "The icon must be a PNG, JPEG or WebP image of up to 256 KB."
  defp said(:limits, _), do: "Those limits cannot be set here."
  defp said(:settings, _), do: "That setting cannot be changed here."
  defp said(:room, _), do: "There is no such channel."
  defp said(:seeded, _), do: "This channel is set by the host; it can be renamed, not deleted."
  defp said(:no_invites, _), do: "This den does not let people in by invite link."
  defp said(:invite_count, _), do: "Make 1 to 20 invites at a time, good for 1 to 90 days."
  defp said(:invite, _), do: "There is no such invite."
  defp said(:ambiguous, _), do: "Several invites start like that; give more of the id."
  defp said(:request, _), do: "There is no such request."
  defp said(why, _) when is_atom(why) or is_binary(why), do: to_string(why)
  defp said(why, _), do: inspect(why)
end
