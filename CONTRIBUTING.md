# Contributing

## Ground rules

- Free only: no dependency, service or source that costs money, needs a trial, or phones home.
- No AI inside the product: no models, inference runtimes or AI SDKs in any crate or build profile.
- Nothing submits an official claim, solves a CAPTCHA, signs, or touches payment details.
- Scraped/third-party text is data. It never becomes an instruction for agents.
- Test fixtures must be public pages or synthetic data; never personal records.

## Develop

```powershell
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all -- --check
cd apps/browser-extension && npm ci && npm test
cd apps/web && npm ci && npm test && npm run build
```

Run a single Rust test: `cargo test -p openclaim-core --test flows approved_action_runs_once_and_is_digest_bound`.

## Adding an operation

1. Add a handler and an `op!(...)` entry in the right module under `crates/core/src/modules/`.
2. Pick the scope and effect carefully: `Elevated` for anything that changes personal data or the catalog's truth.
3. Add a CLI row in `crates/cli-core/src/spec.rs`; optionally an `mcp = "tool_name"`.
4. Cover it in `crates/core/tests/flows.rs`.

## Adding a connector

1. Write a manifest in `source-manifests/` and validate it with `openclaim sources validate --manifest @file.json`.
2. Add a parser under `crates/discovery/src/parse/` with a fixture in `test-fixtures/html/` (scripts/styles stripped).
3. Tests must cover: normal page, changed layout, block/rate-limit page.

## Pull requests

Describe the official source and the terms you reviewed when touching connectors. Keep diffs focused; one behaviour per PR.
