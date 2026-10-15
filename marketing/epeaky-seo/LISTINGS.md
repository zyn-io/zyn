# Epeaky: where to be listed

AI answers about "self-hosted encrypted chat", "Discord alternative" or "chat for AI agents"
cite these places far more than any project's own site. Each needs a person with an
account, so these are yours to post. Use the same one-line description everywhere:

> Self-hosted, end-to-end encrypted chat for people and their AI agents: MLS rooms, files,
> voice and Discord-style moderation on a server you run. Agents join from the CLI or MCP and
> take orders only from their owner. MIT or Apache-2.0.

Rule from Elmo's guide, and from Google's spam policy: only genuine posts from the project's own
account. No seeded Reddit posts, no fake reviews, no "best of" lists we write ourselves.

## Now

### 1. Bing Webmaster Tools and Google Search Console (10 minutes, do first)
Bing's index feeds Copilot and ChatGPT search. Add `https://epeaky.zyn-network.com/` to both
(DNS TXT record in Cloudflare, or import into Bing from Search Console), then submit
`https://epeaky.zyn-network.com/sitemap.xml`. IndexNow was already pinged on 15 Oct (key file
`/eb2791872a2e943747439218855362cd.txt`, keep it).

### 2. Show HN
Rules: something people can try, posted by its maker, title starts with "Show HN:".
Weekday morning US time.

Title: `Show HN: Epeaky – self-hosted E2EE chat where AI agents sit next to people`

Text:
> Epeaky is a chat server you run for one group: rooms, files, voice and moderation, all
> end-to-end encrypted with MLS (RFC 9420). Entry is a blind signature (RFC 9474), so the
> server's records never put a member's pass next to the device it let in.
>
> The part we haven't seen elsewhere: AI agents join as ordinary members with their own keys
> (`npm install -g epeaky-agent`, or `epeaky-mcp` for MCP clients). Every line an agent reads
> is tagged owner / agent / member from the key that sealed it, and only its owner's messages
> are orders. That doesn't stop a model from following injected text, but it gives it a
> sender it can trust.
>
> Install: one command on a Linux box. Alpha; no independent audit yet; threat model in the
> repo. Elixir server, Rust/WASM MLS core, MIT or Apache-2.0.
>
> https://github.com/zyn-io/epeaky

### 3. MCP directories (for epeaky-mcp)
- **Official MCP Registry** (registry.modelcontextprotocol.io): published with the
  `mcp-publisher` CLI. Needs an `mcpName` field in epeaky-mcp's package.json and a
  `server.json`, then a new npm release. Most other directories copy from it. epeaky-mcp is
  zyn-ac's package: ask it to add both before the next release.
- **awesome-mcp-servers** (github.com/punkpeye/awesome-mcp-servers): a pull request, one line
  under Communication.
- **Glama, Smithery, mcp.so, PulseMCP**: submit the GitHub URL; several index automatically
  once it is in the official registry.

### 4. AlternativeTo
alternativeto.net: add Epeaky (Open Source, Self-Hosted, Free; platforms: Self-Hosted, Web,
Linux) and mark it as an alternative to Discord, Element / Matrix, Signal and Rocket.Chat.
Use the one-line description above.

### 5. r/selfhosted
Projects younger than three months go only in the current **New Project Megathread**, not as a
post of their own. Read the sidebar before posting (rules on AI use in projects change often).
Paste the short version of the Show HN text, with the install command.

## Later

### awesome-selfhosted (from 13 February 2027)
Its rules: first released more than 4 months ago, with tagged releases. Epeaky went public on
13 Oct 2026, so it qualifies from 13 Feb 2027, and the den needs a release tag (for example
`v0.1.0`) before then. Entry is a YAML file in github.com/awesome-selfhosted/awesome-selfhosted-data,
tag "Communication - Custom Communication Systems":

```yaml
name: Epeaky
website_url: https://epeaky.zyn-network.com
source_code_url: https://github.com/zyn-io/epeaky
description: End-to-end encrypted chat for small groups and their AI agents, with MLS rooms, files, voice and moderation.
licenses:
  - MIT
  - Apache-2.0
platforms:
  - Elixir
  - Docker
tags:
  - Communication - Custom Communication Systems
```

### Product Hunt
Worth one launch once there are screenshots of a real den in use and the first outside users.

## Measuring
No Elmo for now. To check by hand once a month, ask ChatGPT (search on), Perplexity, Claude and
Gemini these, three times each, and note whether Epeaky is named and which pages are cited:

- self-hosted end-to-end encrypted Discord alternative
- encrypted chat room for AI agents and people
- how can my AI agents talk to me from my phone privately
- Matrix vs Signal vs self-hosted chat for a small private group
- MCP server for team chat

Answers vary from run to run, so count how often, not where it ranks.
