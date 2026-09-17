# How Grayscale’s ZCSH ETP actually works

An original vector explainer for the ZecHub visual bounty, dated September 8, 2026.

## Deliverables

- `source/zcsh-etp.editable.svg` — master artwork, 1600 × 1760, live editable text, embedded fonts, named Inkscape layers.
- `exports/zcsh-etp.svg` — portable vector export; all visible text outlined, no external resources or raster images.
- `exports/zcsh-etp.png` — high-resolution PNG, 3200 × 3520.
- `exports/zcsh-etp-preview.png` — compact preview, 800 × 880.
- `source/build.py` — reproducible artwork generator with editable copy and layout coordinates.
- `SOURCES.md` — primary sources, date qualifications and brand attribution.

Open the master in a browser or vector editor. For editors that ignore embedded SVG fonts, install the included DM Sans and Syne font files. Use the outlined SVG for stable display or distribution.

## Visual language

White background, ZecHub blue structure, Zcash yellow holdings, and clear role labels. Solid unarrowed lines connect service providers to the fund. The dashed line identifies the listing relationship. Double-headed arrows identify basket activity and share trading; they do not imply retail redemption or in-kind redemption.

## Rebuild

Requires Python 3.10+ and a system Cairo library. On macOS, Cairo can be installed with `brew install cairo`.

```sh
python3 -m venv .venv
.venv/bin/pip install -r source/requirements.txt
.venv/bin/python source/build.py
```

The editable SVG can also be changed directly without running the build. Changes made directly in the SVG are not reflected back into the generator.

## Suggested submission text

One vector diagram explaining the conversion of Grayscale’s Zcash Trust into ZCSH, its ZEC holdings, the roles of Coinbase Custody and BNY, and the difference between AP basket activity and investors trading shares on NYSE Arca. Includes an editable SVG, outlined SVG, PNG, and dated primary-source notes. Uses ZecHub’s official logo and blue, the official Zcash mark, and typography referenced from the wiki.

## Alt text

Grayscale Zcash Trust converted into ZCSH, The Zcash ETF. Grayscale sponsors the fund, which holds real ZEC. Coinbase Custody safekeeps its ZEC and BNY handles accounting and records. Jane Street Capital and Virtu Americas create and redeem share baskets. ZCSH shares are listed on NYSE Arca, where investors buy and sell them through brokerage accounts. Investors own fund shares rather than ZEC in a personal wallet. AP redemptions are cash-only under the August 24, 2026 prospectus. Grayscale launched the product; Zcash is the network behind the asset.

Created for review; not submitted or published. Brand assets remain the property of their respective owners. Font licenses are included in `assets/`.
