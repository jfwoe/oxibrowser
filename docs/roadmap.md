# OxiBrowser Roadmap

> Living roadmap. Historical milestone plans (v0.5 era) live in git history;
> dated design docs live in `designs/`.

## Current state (v0.22)

| | |
|---|---|
| Crates | 4 (`oxibrowser`, `oxibrowser-cdp`, `oxibrowser-core`, `oxibrowser-render`) |
| Rust LOC | ~52k |
| Tests | ~660 (unit + CDP e2e + acceptance harness); real-website suite `--ignored` |
| Render stack | Blitz (Stylo CSS, Taffy layout, Parley text, vello_cpu paint) |
| JS | boa_engine 0.20 (ES2024+), wasmi WASM bridge, dedicated JS thread |
| Protocols | CDP (WebSocket), Puppeteer/Playwright compatible |
| Anti-bot | wreq TLS fingerprint impersonation, stealth navigator surface, challenge handling |

Recently shipped (v0.22.0):

- `Page.startScreencast`/`stopScreencast`/`screencastFrameAck` — generation-token
  frame suppression; DOM-mutation bindings journal before applying, so only real
  document changes emit frames.
- `OXI.getInteractiveElements` — document-order interactive elements with
  computed roles and unique CSS selector paths (browser-use workflows).
- `matchMedia`, `DOMParser`, `structuredClone`, `requestIdleCallback` in the JS
  runtime.
- `Page.printToPDF` — real multi-page pagination (A4/Letter, orientation,
  margins) via `oxibrowser_render::png_to_pdf_paged`.
- JS-thread hardening — live element objects serialize safely as eval results
  (tree accessors and `children`/`parentNode` are non-enumerable; no more
  `JSON.stringify` stack overflow).

## Next

Prioritized by agent-workload impact:

1. **Public compatibility benchmark** — a committed corpus runner (WPT subset or
   fixed public-URL set) aggregated in CI, publishing a pass-rate number.
   Everything else on this list is judged by it.
2. **ReadableStream** — completes `fetch().body`; frequent in modern scripts.
3. **Web platform gaps, small batch** — `crypto.subtle`, `history` breadth,
   `requestSubmit`, form validation API.
4. **IndexedDB** — new crate (leveldb-style store) bridged into the JS thread,
   mirroring the localStorage pattern.
5. **Web Workers** — dedicated + shared worker contexts over the existing
   per-frame context machinery.
6. **WebDriver BiDi** — second automation protocol on the same serve endpoint.
7. **Precise mutation epoch** — extend generation journaling to direct property
   setters that bypass the mutation vec (residual: screencast misses those
   rare paths today).

## Non-goals

- V8 or any C/C++ JS engine (identity: pure-Rust, small binary, fast cold start).
- GUI window / GPU compositor / pixel-perfect Chrome parity.
- Workspace fragmentation beyond 4 crates (contribution surface stays small).
