# OxiBrowser Roadmap

> Living roadmap. Historical milestone plans (v0.5 era) live in git history;
> dated design docs live in `designs/`.

## Current state (v0.23)

| | |
|---|---|
| Crates | 4 (`oxibrowser`, `oxibrowser-cdp`, `oxibrowser-core`, `oxibrowser-render`) |
| Rust LOC | ~52k |
| Tests | ~660 (unit + CDP e2e + acceptance harness); real-website suite `--ignored` |
| Render stack | Blitz (Stylo CSS, Taffy layout, Parley text, vello_cpu paint) |
| JS | boa_engine 0.20 (ES2024+), wasmi WASM bridge, dedicated JS thread |
| Protocols | CDP (WebSocket), Puppeteer/Playwright compatible |
| Anti-bot | wreq TLS fingerprint impersonation, stealth navigator surface, challenge handling |
| Security | HAR/CDP-event redaction by default, password masking + capture guard, JSONL audit log, origin-policy primitives |

Recently shipped (v0.23.0):

- Secret redaction on by default — `--har` and CDP network event URLs scrub
  auth headers and sensitive query/body values; `--har-raw` is an audited
  opt-out, `--redact-header` extends the list. See
  `designs/2026-09-27-agent-auth-implementation.md` (P0/M0.1–M0.4 + M1).
- Password masking in DOM observations and a screenshot/PDF capture guard
  while a password field has focus.
- JSONL audit log (`--audit`/`--no-audit`) with handle+fingerprint credential
  references — no secret values in logs.
- `network::origin_policy` — exact-origin matching, DNS label-boundary
  checks, fail-closed frame-origin evaluation (foundation for the credential
  broker, P1).

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
