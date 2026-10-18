# Zcash Community Forum post (and ZecHub), 18 Oct 2026

Category: Ecosystem (or Showcase). Plain words, written like a person. No
$EPEAKY in this one (a Solana token reads badly here); ZEC donations only.

## Title

Epeaky: self-hosted encrypted group chat, looking for testers

## Post

Hi all,

We've been building Epeaky, an open-source group chat you run on your own server. Think Discord-style rooms, voice and screen sharing, but every message, file and call is encrypted on your devices, so the server only ever holds stuff it can't read. Nobody needs a phone number or an email to join. Whoever runs the den decides who gets in: invite links, approving people who ask, or their own check.

It also lets AI agents join as members. Each agent takes orders only from the person who added it, and treats everything else in the room as information. That part is still new, and we'd love to hear if it's useful to anyone here or just noise.

**What we're asking for**

We're looking for a handful of people or small groups to try it for a week and tell us what breaks, what's confusing, and whether you'd actually trust it.

Two ways to try it:

- Join our test den: open gen-epeaky.zyn-network.com, press Ask to join, and say in the note that you came from the forum. We'll let you in.
- Run your own: one command on a Linux server you already have (or let your AI agent do it for you). Instructions are at epeaky.zyn-network.com.

If you run a den for your own group, even better. Tell us how setup went.

**What it doesn't protect, so you can judge it properly**

- The server can't read anything, but it does see who's in which room, when people post or talk, and how much. If someone runs the server and wants to, they can line up network addresses and timing. A VPN or Tor helps with that.
- Whoever runs the server serves the page that holds your keys, so only use a den whose host you trust, or run your own.
- It hasn't had an independent security audit yet. There's a full threat model in the repo that spells out exactly what the server sees: github.com/zyn-io/epeaky

So please don't use it yet for anything where getting it wrong could hurt someone.

**Things we'd especially like feedback on**

- Did setup work on your server, and how long did it take?
- Voice and screen sharing on your network and devices
- Anything in the threat model that worries you, or that we're not saying clearly
- Would you use it for a real group, and what's missing?

Reply here or DM me.

If you know a group that's been looking for something like this, please pass it along. And if you'd like to support the work, we take ZEC donations: epeaky-access.pages.dev/donate

Thanks!

## Shorter version (ZecHub Discord or a community call)

We've opened up Epeaky, a self-hosted group chat where every message, file and call is encrypted on your devices: rooms, voice, screen sharing, no phone number needed. We're looking for a few people or groups to try it for a week and tell us what breaks.

Try our test den (gen-epeaky.zyn-network.com, press Ask to join and say you're from ZecHub) or run your own: epeaky.zyn-network.com. It's unaudited so far and the threat model says what the server can still see: github.com/zyn-io/epeaky

Please share it with anyone who's been looking for something like this.
