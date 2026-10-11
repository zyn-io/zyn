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
    options "/*any", DoorController, :keys
  end
end
