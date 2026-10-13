defmodule DenWeb.Router do
  use DenWeb, :router

  pipeline :api do
    plug DenWeb.Cors
    plug :accepts, ["json"]
  end

  pipeline :blob do
    plug DenWeb.Cors
  end

  scope "/blob", DenWeb do
    pipe_through :blob
    post "/", BlobController, :create
    get "/:id", BlobController, :show
    options "/*any", BlobController, :show
  end

  scope "/door", DenWeb do
    pipe_through :api
    get "/keys", DoorController, :keys
    post "/sign", DoorController, :sign
    post "/enter", DoorController, :enter
    # The ways in this den opens (Den.Gate.Members).
    get "/ways", DoorController, :ways
    post "/join", DoorController, :join
    post "/ask", DoorController, :ask
    get "/ask/:id", DoorController, :asked
    post "/passes", DoorController, :register
    delete "/passes/:pass", DoorController, :remove
    options "/*any", DoorController, :keys
  end
end
