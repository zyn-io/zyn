defmodule Den.Public do
  @moduledoc """
  Each room's public MLS state, kept by the den and checked by OpenMLS's
  `PublicGroup` (native/den_public): what a Delivery Service may hold.

  Members send commits as signed PublicMessages. The den orders a commit only
  if it verifies against the room's state, and learns who signed it; it takes
  a GroupInfo only if it describes that same state. Application messages stay
  encrypted. State is the NIF's JSON, kept on the room.
  """
  use Rustler, otp_app: :den, crate: "den_public"

  @doc "Start tracking a room from its first GroupInfo: {:ok, {state, room, epoch}}."
  def track(_group_info), do: :erlang.nif_error(:nif_not_loaded)

  @doc "Check and apply a commit: {:ok, {state, epoch, signer_key_hex}}."
  def commit(_state, _message), do: :erlang.nif_error(:nif_not_loaded)

  @doc "Whether a GroupInfo describes the tracked state: {:ok, boolean}."
  def info_matches(_state, _group_info), do: :erlang.nif_error(:nif_not_loaded)
end
