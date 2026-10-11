defmodule Den.Mls do
  @moduledoc """
  What the den can read of an MLS message (RFC 9420 §6) without any key: the
  room (group id), the epoch, and whether it is a commit — all that ordering
  needs. A member's messages and commits are PrivateMessages, whose content
  stays sealed; an external join is a PublicMessage, as MLS requires.
  """

  @public 1
  @private 2
  @commit 3

  @doc "%{room, epoch, commit, external} | {:error, reason}"
  def envelope(<<1::16, @private::16, rest::binary>>) do
    with {:ok, room, rest} <- vec(rest),
         <<epoch::64, type::8, _::binary>> <- rest do
      {:ok, %{room: room, epoch: epoch, commit: type == @commit, external: false}}
    else
      _ -> {:error, :malformed}
    end
  end

  def envelope(<<1::16, @public::16, rest::binary>>) do
    with {:ok, room, rest} <- vec(rest),
         <<epoch::64, sender_type::8, rest::binary>> <- rest,
         {:ok, rest} <- sender(sender_type, rest),
         {:ok, _aad, rest} <- vec(rest),
         <<type::8, _::binary>> <- rest do
      {:ok, %{room: room, epoch: epoch, commit: type == @commit, external: sender_type == 4}}
    else
      _ -> {:error, :malformed}
    end
  end

  def envelope(_), do: {:error, :not_a_room_message}

  # member (1) and external (2) senders carry a u32 index; new members (3, 4) nothing.
  defp sender(t, <<_::32, rest::binary>>) when t in [1, 2], do: {:ok, rest}
  defp sender(t, rest) when t in [3, 4], do: {:ok, rest}
  defp sender(_, _), do: :error

  # An opaque<V>: a QUIC-style variable-length length, then the bytes.
  defp vec(<<0::2, n::6, rest::binary>>), do: take(n, rest)
  defp vec(<<1::2, n::14, rest::binary>>), do: take(n, rest)
  defp vec(<<2::2, n::30, rest::binary>>), do: take(n, rest)
  defp vec(_), do: :error

  defp take(n, bin) when byte_size(bin) >= n do
    <<v::binary-size(n), rest::binary>> = bin
    {:ok, v, rest}
  end

  defp take(_, _), do: :error
end
