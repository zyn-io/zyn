defmodule DenWeb.Origins do
  @moduledoc "Pages allowed to open a socket: the Cave's own."
  def allowed?(%URI{} = uri) do
    origin = "#{uri.scheme}://#{uri.host}" <> if(uri.port in [80, 443, nil], do: "", else: ":#{uri.port}")
    origin in Application.get_env(:den, :origins, [])
  end
end
