# The den: an Elixir release on Debian. A host's add-on modules in extra/ are
# compiled in when the folder is there (mix.exs); the page in priv/page goes in
# with priv.
ARG ELIXIR=1.19.5
ARG OTP=28.4.3
ARG DEBIAN=bookworm-20260824-slim

FROM hexpm/elixir:${ELIXIR}-erlang-${OTP}-debian-${DEBIAN} AS build
# ex_webrtc's native parts (voice): DTLS over OpenSSL, SRTP over libsrtp.
RUN apt-get update -y && apt-get install -y build-essential git pkg-config libssl-dev libsrtp2-dev curl ca-certificates && rm -rf /var/lib/apt/lists/*
# The rooms' public-state NIF (native/den_public) is Rust, built by rustler.
RUN curl -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal --default-toolchain 1.93.1
ENV PATH=/root/.cargo/bin:$PATH
WORKDIR /app
ENV MIX_ENV=prod
RUN mix local.hex --force && mix local.rebar --force
COPY mix.exs mix.lock ./
RUN mix deps.get --only prod && mkdir config
COPY config/config.exs config/prod.exs config/
RUN mix deps.compile
COPY priv priv
COPY native native
COPY lib lib
COPY extra extra
RUN mix compile
COPY config/runtime.exs config/
# The release environment: Erlang distribution on loopback, each den its own cookie.
COPY rel rel
RUN mix release

FROM debian:bookworm-20260824-slim
RUN apt-get update -y && apt-get install -y libstdc++6 openssl libssl3 libsrtp2-1 libncurses6 locales ca-certificates && rm -rf /var/lib/apt/lists/* \
  && sed -i '/en_US.UTF-8/s/^# //g' /etc/locale.gen && locale-gen
ENV LANG=en_US.UTF-8 LANGUAGE=en_US:en LC_ALL=en_US.UTF-8 MIX_ENV=prod PHX_SERVER=true
WORKDIR /app
RUN useradd --uid 9100 --system den && mkdir -p /var/lib/den && chown den /var/lib/den
COPY --from=build --chown=den /app/_build/prod/rel/den ./
USER den
CMD ["/app/bin/den", "start"]
