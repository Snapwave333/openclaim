# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Current state

No code exists yet. The only source of truth is `OpenClaim_Master_Design_Record_v1.1.md` (the "MDR"). Section numbers below refer to it. Every command, crate, endpoint and CLI subcommand in the MDR is a **target spec**, not existing software. When you scaffold something, update this file with the real commands. The folder is not a git repository yet.

Build order is the MDR roadmap (§15): Phase 0 foundation (Rust workspace, Angular app, `openclaim` CLI, `openclaimd` daemon, SQLite migrations, local IPC, CI) → Phase 1 FTC/SEC connectors and discovery → Phase 2 rules engine, profiles, evidence vault → Phase 3 filing assistance → Phase 4 full CLI/MCP agent control → Phase 5 hardened release.

## Target commands (§23, adopt once scaffolded)

```bash
cargo test --workspace
cargo test -p <crate> <test_name>          # single Rust test
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all -- --check
npm ci
npm run test --workspace=apps/web
npm run build --workspace=apps/web
cargo build --release --bin openclaim --bin openclaimd
```

## Architecture (§6, §7)

- **`openclaimd`** (Rust, Tokio/Axum) is the single source of truth and the **only SQLite writer** (WAL + FTS5). It owns discovery, rules, evidence vault, claims, jobs, scheduler, approvals and audit.
- **`openclaim` CLI** (clap v4) and the **MCP adapter** (`openclaim mcp serve --transport stdio`) talk to the daemon over authenticated local IPC: a Windows named pipe with an owner-only ACL, or a 0600 Unix socket. They never open the database directly.
- **Angular GUI** (`apps/web`, strict TS, standalone components, signals) is an optional client over localhost REST + SSE, bound to `127.0.0.1` with Origin checks, a CSRF/session secret and a CORS allowlist. Closing it never cancels daemon jobs.
- CLI, GUI and MCP call the **same typed domain services**. Any non-visual workflow added to the GUI needs a CLI equivalent (and an MCP tool where §19.2 lists one).
- Planned layout: `apps/{desktop,web,cli,browser-extension}`, `crates/{api,domain,db,discovery,extract,matching,claims,notifications,jobs,ipc,cli-core,mcp-adapter,agents,security}`, `migrations/`, `source-manifests/`, `test-fixtures/` (public or synthetic data only).
- Data model: §8. Money uses integer minor units, never floats.

## Hard constraints (release gates, §3, §14, §24)

- **No AI inside the product.** No LLM, inference runtime, AI SDK, embeddings, Ollama/llama.cpp/ONNX, or model setting in any binary or build profile. Search is FTS5 + deterministic ranking. Matching is a deterministic three-valued rules AST (§9).
- **Free only.** Every dependency and source must be $0 and open source. No paid APIs, PACER, paid OCR, captcha solvers, proxies, analytics or payment processors. Each connector manifest declares `price=free`, `auth_requirement`, `robots_policy`, `terms_review_date`, `rate_limit`, `automation_allowed`, and auto-disables if access becomes paid.
- **No automatic claim submission.** CAPTCHA, identity verification, signatures, payment details and the final Submit always stay with the user. There is no `claims.submit_official` capability.
- **No fabricated data.** Never insert a placeholder as a live settlement. Unverified opportunities are labeled `verification_pending` and excluded from verified-only views. Eligibility output is only `potential_match | needs_more_information | unlikely | not_applicable | manual_review`, never "eligible", and always with cited reasons.
- **Agent-proposed facts are untrusted** (`proposed_facts` table) until a human confirms them. Agents get read-only research scope by default. Approvals are digest-bound, expire, and never auto-approve (§20).
- **Scraped content is untrusted data.** It must never be able to add tools, change scopes or instruct agents through tool output. SSRF defenses on every fetch (§12).
- Respect robots rules, terms, rate limits and `Retry-After`. A blocked source falls back to an official manual link; it is never bypassed.

## CLI contract (§16–§18, §22)

- `--output table|text|json|ndjson`. In `json` mode stdout is exactly one envelope (`schema_version`, `ok`, `request_id`, `trace_id`, `data` | `error`, ...). In `ndjson` mode stdout is one event per line. Banners, progress, color and diagnostics go to stderr only.
- `--dry-run` never mutates, queues jobs, imports files or makes side-effecting requests.
- `--non-interactive` returns `APPROVAL_REQUIRED` (exit 5) with an `approval_id` instead of prompting.
- Exit codes 0/2–11 (§22.1) are public API. A failed mutation never exits 0.
- Long tasks return a durable `job_id`; jobs survive CLI exit, GUI close and daemon restart.
