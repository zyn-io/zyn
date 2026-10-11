defmodule DenWeb.Router do
  use DenWeb, :router

  pipeline :api do
    plug DenWeb.Cors
    plug :accepts, ["json"]
  end

  scope "/door", DenWeb do
    pipe_through :api
    get "/keys", DoorController, :keys
    post "/sign", DoorController, :sign
    post "/enter", DoorController, :enter
    options "/*any", DoorController, :keys
  end
end
