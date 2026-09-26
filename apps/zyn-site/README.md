# zyn-site

Zyn's landing page: plain HTML, CSS and one script, no build step. Live at
https://zyn-site.pages.dev (Cloudflare Pages project `zyn-site`).

    /opt/homebrew/bin/wrangler pages deploy public --project-name zyn-site --branch main

- The ledger table reads ZynScan (`zyn-rpc.cavebro.win/scan*/api/{summary,epochs,txs}`); `_headers` allows only that origin.
- Copy follows `docs/MESSAGING.md`: every claim here is one that page lists, or one checked against the object ledger's code.
- The Z is `brandkit/zyn-mark-jade.png` cut to `assets/zyn-mark-3d*.webp` with its edges faded to black (it is screen-blended onto the page); `assets/og.png` is rendered from the page itself.
- Fonts: Archivo (display, widest cut; OFL), Geist and Geist Mono (OFL), all self-hosted in `assets/fonts`.
- Concept: jade is a stone, and so is the ledger. Epochs are drawn as strata, anchored ones pinned to Zcash bedrock; the verifiable checks are hallmarks. `assets/shots/` are real captures of the live apps (re-capture when they change).
- Motion: `cubes.js` breaks each `[data-cubes]` heading into cubes as its section scrolls away and assembles the next from cubes (scroll-linked, reversible; the footer ZYN is whole exactly at the bottom). `main.js` has the section lock: the page never rests across two sections; it finishes the move in the direction of travel (sections not much taller than a screen are entered at their start), leaves tall sections free inside, and absorbs same-direction wheel during a glide (one gesture, one section). Both are off under reduced motion.
- The CSP allows no inline style attributes: anything dynamic sets styles through the CSSOM (`el.style`).
- When the domain is bought: attach it to the Pages project and update the `og:image` URL in `index.html`.
