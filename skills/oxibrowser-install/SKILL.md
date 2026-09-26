# Skill: Install OxiBrowser

Install the OxiBrowser CLI (pure-Rust headless browser for AI agents) on the
local machine. Follow the steps in order and stop at the first success.

## Step 1 — Check for an existing install

```bash
oxibrowser version
```

If it prints a version, report it and stop.

## Step 2 — Install via cargo (works everywhere with a Rust toolchain)

Requires a C toolchain (cc/clang) and cmake for the TLS backend.

```bash
cargo install oxibrowser
```

Then verify with `oxibrowser version`. On success, stop.

## Step 3 — Try a prebuilt release binary

1. List published releases:

   ```bash
   gh release list --repo project-oxi/oxibrowser --limit 5
   ```

2. If the latest release publishes an asset matching the local platform
   (check `uname -s`/`uname -m`: Darwin+arm64 → aarch64-apple-darwin,
   Linux+x86_64 → x86_64-unknown-linux-gnu, etc.), download it, mark it
   executable, move it onto `PATH`, and re-verify.

3. If no matching asset exists, report that Step 3 was unavailable — do not
   build from source unless the user asked for it.

## Verify the install

```bash
oxibrowser version
oxibrowser fetch https://example.com --format markdown --json --max-bytes 2000
```

The fetch should return JSON with markdown content for example.com.

## Notes

- Do not use `sudo`.
- Do not modify shell rc files; report the installed path instead.
