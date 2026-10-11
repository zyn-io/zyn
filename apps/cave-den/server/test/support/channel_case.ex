defmodule DenWeb.ChannelCase do
  @moduledoc "Channel tests, each in its own sandboxed database transaction."
  use ExUnit.CaseTemplate

  using do
    quote do
      import Phoenix.ChannelTest
      import DenWeb.ChannelCase
      @endpoint DenWeb.Endpoint
    end
  end

  setup tags do
    Den.DataCase.setup_sandbox(tags)
    :ok
  end
end
