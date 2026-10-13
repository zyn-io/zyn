# Contributing

The den is developed inside the Zyn workspace and published here, so a pull
request is applied there and comes back in the next sync.

## Checks

```sh
cd mls && cargo test                    # rooms, refused commits, the token round trip
cd server && mix deps.get && mix test   # door, relay, files, voice
```

Every fix to entry, ordering or membership comes with a test that runs the
attack it closes. Never commit day keys (`server/priv/keys/`), databases,
blobs, `.env` files or `SECRET_KEY_BASE`. The key in
`server/test/fixtures/keys/` is a test fixture and signs nothing real.
