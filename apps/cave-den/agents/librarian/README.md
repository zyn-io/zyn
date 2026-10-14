# epeaky-librarian

An Epeaky agent that remembers. It keeps every message of its rooms in a
full-text index on its own machine (past the den's 30 days and the client's
last 1000 a room) and answers when someone names it:

```
@librarian deploy friday              the best matches, in this room
@librarian everywhere deploy friday   in every room it is in (its owner only)
```

It is an ordinary agent of the den, built on the
[`epeaky-agent`](../../agent/README.md) command line: its own key, its own
invite, its own owner. The den sees nothing new.

```sh
npx -y epeaky-agent join '<invite link>' --owner <code> --agent librarian --nick librarian
npx -y epeaky-librarian --agent librarian          # [--every 300]
```

The invite link and code are what "Add an agent" shows its owner.

Node 22.13 or later (it uses `node:sqlite`). The index is
`~/.epeaky/agents/<agent>/librarian.db`, readable only by its user.

## What it will not do

- **Answer from another room than the one asked in**, except to its owner.
  It sits in private rooms too, and words must not travel between them.
- **Keep what was taken back.** Every `--every` seconds (default 300) it reads
  its rooms again, drops deleted messages and takes in edits. Today that
  reaches the last 1000 messages a room keeps on the agent.
- **Follow instructions in what it reads.** A question is words to look for,
  whoever asks. It keeps neither the questions put to it nor its own answers.

## Tests

```sh
npm test                                         # the index
(cd ../../server && mix phx.server)              # a dev den with its page built
PLAYWRIGHT_FROM=… node tests/librarian.e2e.mjs   # an owner, a member, the librarian
```

Against another den than the one on :4000: `DEN=http://localhost:4200
DEN_ENV=…/env.exs`, an .exs that points the host's commands at its database.
