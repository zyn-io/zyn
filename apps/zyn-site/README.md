# zyn-site

Zyn's landing page: plain HTML, CSS and one script, no build step. Live at
https://zyn-network.com (and www.; Cloudflare Pages project `zyn-site`, also
zyn-site.pages.dev). The lending app is https://lend.zyn-network.com (project
`zyn-lend`, repo zyn-io/zyn-lend).

    /opt/homebrew/bin/wrangler pages deploy public --project-name zyn-site --branch main

- The ledger table reads ZynScan (`zyn-rpc.cavebro.win/scan*/api/{summary,epochs,txs}`); `_headers` allows only that origin.
- The FAQ (ten questions) answers inside the same limits as the trust section: operator-sequenced, custodian-dependent withdrawals, privacy from the public but not the operator, test bridges on one machine, no permissionless deploys.
- Copy follows `docs/MESSAGING.md`: every claim here is one that page lists, or one checked against the object ledger's code.
- The Z is `brandkit/zyn-mark-jade.png` cut to `assets/zyn-mark-3d*.webp` with its edges faded to black (it is screen-blended onto the page); `assets/og.png` is rendered from the page itself.
- Fonts: Archivo (display, widest cut; OFL), Geist and Geist Mono (OFL), all self-hosted in `assets/fonts`.
- Concept: jade is a stone, and so is the ledger. Epochs are drawn as strata, anchored ones pinned to Zcash bedrock; the verifiable checks are hallmarks. `assets/shots/` are real captures of the live apps (re-capture when they change).
- Motion: `cubes.js` breaks each `[data-cubes]` heading into cubes as its section scrolls away and assembles the next from cubes; scroll decides when, each plays on its own clock (1.4 s apart, 1.6 s together) so the section glide does not rush it; the footer ZYN forms at the bottom. `main.js` has the section lock: the page never rests across two sections; it finishes the move in the direction of travel (sections not much taller than a screen are entered at their start), leaves tall sections free inside, and absorbs same-direction wheel during a glide (one gesture, one section). Both are off under reduced motion.
- The CSP allows no inline style attributes: anything dynamic sets styles through the CSSOM (`el.style`).
- The domain (bought 30 Sep at Cloudflare Registrar) is attached to the Pages projects as custom domains; `og:image` points at it.
