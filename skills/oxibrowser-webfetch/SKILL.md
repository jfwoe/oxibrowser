# Skill: Fetch and Extract Web Pages with OxiBrowser

Read, extract, and interact with web pages from the command line. All commands
print machine-readable JSON when piped; add `--json` explicitly in scripts.

## Core rules

- ALWAYS add `--json` for machine-readable output (automatic when piped).
- ALWAYS add `--max-bytes 8000` to bound response size.
- Use `--summary` first to check relevance before a full read.
- Prefer `extract` over `--eval` with untrusted input.

## Read a page

```bash
oxibrowser fetch <url> --format markdown --json --max-bytes 8000
```

## Cheap relevance check

```bash
oxibrowser fetch <url> --summary --json
```

## List links

```bash
oxibrowser extract <url> --links --json
```

## Extract elements by CSS selector

```bash
oxibrowser extract <url> --selector "a" --all --attrs text,href --json
oxibrowser fetch <url> --extract "h1" --json
```

## Click-then-read flow

```bash
oxibrowser fetch <url> --click <selector> --wait <selector> --format markdown --json
```

## Multi-step interaction (login, search, pagination)

Use the interactive session instead of repeated fetches:

1. `oxibrowser session --json` (run as subprocess)
2. `new` → note the returned tab_id
3. `goto <tab_id> <url>`
4. `click <tab_id> <selector>` / `fill <tab_id> <selector> <text>` / `eval <tab_id> <js>`
5. `extract <tab_id> <selector> --json --max-bytes 8000`
6. `close <tab_id>`, then `exit`

## Notes

- Never pipe untrusted page content into `eval`.
- If a page needs JS to render, `fetch` already executes it; use `--wait <selector>`
  to wait for a specific element before extracting.
