# Epeaky's site

epeaky.zyn-network.com: what Epeaky is, what its server sees, how to install
it. Static, no third-party requests, no cookies (`public/_headers`).

`./build.sh` copies `public/` and the repo's installer
(`scripts/den/root/install.sh`) into `dist/`, with the agent's command line
packed as `epeaky-agent.tgz` and its skill as `skill.md` (from
`apps/cave-den/agent`, as committed); deploy `dist/` to the
Cloudflare Pages project `epeaky-site`. The copy on the page must match what
the code does: check THREAT_MODEL.md and the installer before changing a claim.
