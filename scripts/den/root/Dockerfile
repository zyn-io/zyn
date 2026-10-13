# Epeaky: a whole den in one image — the MLS library built to WebAssembly,
# the page, and the server. From the repository's root:
#
#   docker build -t epeaky .
#
# A host's own modules in server/extra/lib are compiled in (see its README).
ARG ELIXIR=1.19.5
ARG OTP=28.4.3
ARG DEBIAN=bookworm-20260824-slim

FROM hexpm/elixir:${ELIXIR}-erlang-${OTP}-debian-${DEBIAN} AS build
# Voice (ex_webrtc): DTLS over OpenSSL, SRTP over libsrtp.
RUN apt-get update -y && apt-get install -y build-essential git pkg-config libssl-dev libsrtp2-dev curl ca-certificates && rm -rf /var/lib/apt/lists/*
# Rust: the rooms' public-state NIF, and the page's MLS library (wasm).
RUN curl -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal --default-toolchain 1.93.1 --target wasm32-unknown-unknown
ENV PATH=/root/.cargo/bin:$PATH
RUN cargo install wasm-bindgen-cli --version 0.2.128 --locked

WORKDIR /src
COPY mls mls
RUN cd mls && ./build.sh

WORKDIR /src/server
ENV MIX_ENV=prod
RUN mix local.hex --force && mix local.rebar --force
COPY server/mix.exs server/mix.lock ./
RUN mix deps.get --only prod && mkdir config
COPY server/config/config.exs server/config/prod.exs config/
RUN mix deps.compile
COPY server/priv priv
COPY web /src/web
# The page, with the Phoenix client from these deps.
RUN /src/web/build.sh /src/server/priv/page
COPY server/native native
COPY server/lib lib
COPY server/extra extra
RUN mix compile
COPY server/config/runtime.exs config/
# The release environment: Erlang distribution on loopback, each den its own cookie.
COPY server/rel rel
RUN mix release

FROM debian:${DEBIAN}
RUN apt-get update -y && apt-get install -y libstdc++6 openssl libssl3 libsrtp2-1 libncurses6 locales ca-certificates && rm -rf /var/lib/apt/lists/* \
  && sed -i '/en_US.UTF-8/s/^# //g' /etc/locale.gen && locale-gen
ENV LANG=en_US.UTF-8 LANGUAGE=en_US:en LC_ALL=en_US.UTF-8 MIX_ENV=prod PHX_SERVER=true \
  DATABASE_PATH=/var/lib/den/den.db DEN_KEYS_DIR=/var/lib/den/keys DEN_VOICE_PUBLIC_ONLY=1
WORKDIR /app
RUN useradd --uid 9100 --system den && mkdir -p /var/lib/den && chown den /var/lib/den
COPY --from=build --chown=den /src/server/_build/prod/rel/den ./
USER den
VOLUME /var/lib/den
CMD ["/app/bin/den", "start"]
