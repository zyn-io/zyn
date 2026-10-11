defmodule Den.Pass do
  @moduledoc """
  A member pass (DECISIONS §172.3): the hash of a code a holder's Nap made,
  registered on the ledger against one of their items. Pass hashes are public
  on ZynScan, so the door asks for the code and hashes it itself: knowing the
  hash is not holding the pass.

  The hash is the ledger's `objectvm::dao::pass_of`: SHA-256 over a leaf
  prefix, the domain and the code.
  """

  @domain "objectvm.dao-pass.v1"

  @doc "The pass a 32-byte code makes."
  def of(<<code::binary-size(32)>>), do: :crypto.hash(:sha256, <<0>> <> @domain <> code)

  @doc "A code from its 64 hex characters, or :error."
  def code(hex) when is_binary(hex) and byte_size(hex) == 64 do
    case Base.decode16(hex, case: :lower) do
      {:ok, code} -> {:ok, code}
      :error -> :error
    end
  end

  def code(_), do: :error
end
