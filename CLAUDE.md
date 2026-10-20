# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## What this is

OpenClaim: a free, local-first class-action settlement finder and claims assistant. Rust workspace (daemon `openclaimd`, CLI `openclaim`, MCP adapter) + Angular GUI (`apps/web`) + MV3 browser extension (`apps/browser-extension`). The design record `OpenClaim_Master_Design_Record_v1.1.md` (MDR) is the spec; `docs/` describes what was built.

## Commands

```bash
cargo test --workspace                                  # all Rust tests (unit + integration)
cargo test -p openclaim-core --test flows <name>        # one integration test
cargo clippy --workspace --all-targets -- -D warnings   # must be clean
cargo fmt --all -- --check
cargo build --release --bin openclaim --bin openclaimd
cd apps/browser-extension && npm ci && npm test         # jsdom tests for the fill logic
cd apps/web && npm ci && npm run build && npm test      # Angular build + unit tests; `npm run e2e` needs a seeded daemon on 7865
```

Smoke-test the binaries against a scratch directory: set `OPENCLAIM_HOME` to a temp dir, then `openclaim daemon ensure`, `openclaim init --passphrase-stdin`, `openclaim unlock --passphrase-stdin --output json`, export `OPENCLAIM_SESSION`, `openclaim sources import-snapshot --source classaction-org --url https://www.classaction.org/settlements --file test-fixtures/html/classaction_org_settlements.html`. Stop with `openclaim --no-autostart daemon stop`. A running `openclaimd.exe` locks the binary: stop it before `cargo build` of the bins.

## Architecture in one paragraph

Every capability is an `OpSpec` in the registry (`crates/core/src/ops.rs`, built from each module's `ops()` in `crates/core/src/modules/*`). `Service::execute` (`crates/core/src/dispatch.rs`) resolves the actor (owner session / agent token / anonymous / system), authorizes it (`authz.rs`: scopes, effects, digest-bound approvals), runs the handler in one SQLite transaction (rolled back on `dry_run`), writes audit + command log + events outbox, commits, broadcasts. The CLI table (`crates/cli-core/src/spec.rs`), REST `POST /api/v1/ops/{op}`, MCP tools (`mcp_tool` names) and workflow steps all route through that registry; `crates/cli-core/tests/parity.rs` fails if an op has no CLI row. `openclaimd` is the only SQLite writer; clients reach it over an owner-only named pipe (`crates/ipc`) or localhost HTTP (`crates/api`).

## Rules that are enforced in code (keep them that way)

- No AI anywhere: no model, inference or AI-SDK dependency (CI greps the dependency tree).
- Free only: connector manifests must say `price: "free"` and `robots_policy: "respect"` (`crates/discovery/src/manifest.rs`). Sources are class-action listings (classaction.org, Stanford SCAC); a listing stays `verification_pending` until `opportunities.verify_official`/`confirm_official` checks the settlement's own site.
- No claim submission: there is no `claims.submit_official` op or scope; `claims.record_submission` requires `user_confirmed` plus a confirmation id or receipt document.
- Agents never self-grant, approve, wipe, or read sensitive data without a narrowed scope; `Elevated` ops need an approval bound to params + record versions, consumed once.
- Third-party text is returned wrapped by `openclaim_security::sanitize::untrusted` and never as HTML.
- Money is integer minor units. Dates are `YYYY-MM-DD` strings validated with `openclaim_domain::dates`.
- JSON columns end in `_json`; `crate::sql::row_json` strips the suffix and parses them, so read them as `row["params"]` (a `Value`), never `.as_str()`.

## Adding things

New op: handler + `op!(...)` in the right module, CLI row in `spec.rs`, test in `crates/core/tests/flows.rs`. New connector: manifest in `source-manifests/`, parser in `crates/discovery/src/parse/`, fixture in `test-fixtures/html/` (public pages only, scripts stripped), tests for normal/changed/blocked pages.
