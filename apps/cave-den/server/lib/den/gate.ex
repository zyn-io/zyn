defmodule Den.Gate do
  @moduledoc """
  Who may enter. A gate answers one question about a member pass: is it live?
  The door does everything else the same way whatever the gate: at most three
  day tokens per pass per day, blind signatures, and proof of a device's key
  at entry (`Den.Door`).

  A pass is a 32-byte code that only its member holds. The default gate is
  the den's own member list (`Den.Gate.Members`), filled by invite links,
  the host's approval, or passes the host's site registers. A host may name
  another module with `config :den, gate: Module` (or `DEN_GATE`); it must
  implement this behaviour.
  """

  @doc "A stable id for the pass a code makes (the door counts tokens by it)."
  @callback id(code :: <<_::256>>) :: binary
  @doc "Whether the pass a code makes is live."
  @callback live?(code :: <<_::256>>) :: {:ok, boolean} | {:error, term}

  def module, do: Application.get_env(:den, :gate, Den.Gate.Members)

  def id(code), do: module().id(code)

  def live?(code) do
    case Application.get_env(:den, :gate_check) do
      # Tests stand in for any gate.
      fun when is_function(fun, 1) -> fun.(code)
      _ -> module().live?(code)
    end
  end

  @doc "A code from its 64 lower-case hex characters, or :error."
  def code(hex) when is_binary(hex) and byte_size(hex) == 64 do
    if hex =~ ~r/\A[0-9a-f]{64}\z/, do: Base.decode16(hex, case: :lower), else: :error
  end

  def code(_), do: :error
end
