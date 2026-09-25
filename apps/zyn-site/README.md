# zyn-site

Zyn's landing page: plain HTML, CSS and one script, no build step. Live at
https://zyn-site.pages.dev (Cloudflare Pages project `zyn-site`).

    /opt/homebrew/bin/wrangler pages deploy public --project-name zyn-site --branch main

- The live strip reads ZynScan's summaries (`zyn-rpc.cavebro.win/scan*/api/summary`); `_headers` allows only that origin.
- Copy follows `docs/MESSAGING.md`: every claim here is one that page lists, or one checked against the object ledger's code.
- The Z is a vector drawn after the brand kit's metallic mark (`brandkit/`); `assets/og.png` is rendered from the page's own styles.
- Fonts: Geist and Geist Mono (OFL, `assets/fonts/Geist-OFL.txt`).
- When the domain is bought: attach it to the Pages project and update the `og:image` URL in `index.html`.
