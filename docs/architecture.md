# Architecture

```
External agent / terminal / browser extension / Angular GUI
        │ CLI (openclaim)        │ MCP stdio          │ REST + SSE (127.0.0.1)
        └────────── authenticated local IPC ──────────┘
                                 ▼
┌──────────────────────── openclaimd ────────────────────────────┐
│ Service::execute → resolve actor → authorize (scopes/approvals)│
│   → one SQLite transaction per command (rolled back on dry run)│
│   → audit + command_runs + events outbox → commit → broadcast  │
│ Jobs: leases, retries, cancellation, crash recovery, schedules │
│ Modules: sources · opportunities · property · people · facts   │
│   documents · matching · claims · alerts · workflows · agents  │
├────────────────────────┬───────────────────────────────────────┤
│ SQLite WAL + FTS5      │ Encrypted vault files (XChaCha20)     │
└────────────────────────┴───────────────────────────────────────┘
```

## Crates

| Crate | Role |
|---|---|
| `domain` | Types, error codes/exit codes, envelope, scopes, deterministic rules engine, claim state machine, dedupe keys. No I/O. |
| `security` | Vault encryption (DPAPI-wrapped key on Windows), tokens/passphrases (argon2), redaction, untrusted-text sanitizing, SSRF/URL policy, canonical JSON digests. |
| `db` | SQLite open/migrate (WAL, FTS5, foreign keys), integrity, backup. |
| `extract` | On-device PDF text, EML/MBOX, CSV/text, optional Tesseract OCR, deterministic fact suggestions with spans. |
| `discovery` | Connector manifests (provider gate), policy-gated fetcher (robots, rate limits, allowlists, redirect checks, bounded bodies), parsers for FTC/SEC/NAUPA/feeds. |
| `core` | The operation registry and everything behind it: dispatch, authorization, approvals, jobs, workflows, audit, every module. |
| `ipc` | Named pipe (owner-only ACL) / Unix socket transport. |
| `api` | Axum REST + SSE + static GUI with Host/Origin/CSRF/CSP defenses. |
| `cli-core` | Data-driven command tree, output contracts, daemon client with explicit auto-start. |
| `mcp-adapter` | Model-free MCP stdio server: tools are registry operations with `mcp_tool` names. |
| `apps/daemon`, `apps/cli` | Binaries. `apps/web` Angular GUI. `apps/browser-extension` MV3 extension. |

## The operation registry

Every capability is an `OpSpec { name, scope, effect, params_schema, handler, mcp_tool }` in `crates/core/src/ops.rs`. The CLI table (`cli-core/src/spec.rs`), REST (`POST /api/v1/ops/{op}`), MCP tools, workflow steps and `schema export` all derive from it, so parity is structural.

Effects: `Read`, `Write` (agents need the scope), `Elevated` (agents need scope **and** a human approval bound to the exact action), `OwnerOnly`.

## Actors

- **Owner**: proved the passphrase (`openclaim unlock` → memory-only session token; GUI login → HttpOnly cookie + CSRF header).
- **Agent**: registered principal with a token and scopes. Cannot self-grant, approve, wipe, or submit.
- **Anonymous**: a local caller without credentials; public research scopes only. Executing the binary grants nothing personal.
- **System**: the scheduler/job runner.

## Approvals

`agent requests → daemon validates → action digest (op + params + principal + record versions) → stored with redacted preview → human approves/denies → agent retries with approval_id → daemon re-checks digest, consumes once → audit`. Expired, denied and invalidated approvals never become approvals.

## Jobs

SQLite-backed queue with leases (`lease_expires_at`), heartbeats via progress updates, exponential retry for retryable errors, cancellation points, crash recovery on start (`recover_jobs`), schedules (sources hourly check honoring per-source TTL, alerts every 15 min, stale sweep 6 h, integrity weekly). Workflow runs are jobs with checkpoints; approval pauses set `paused_for_approval` and resume after the decision.

## Data at rest

- `openclaim.sqlite` (WAL) in `%LOCALAPPDATA%\OpenClaim` (or `OPENCLAIM_HOME`).
- `vault/<id>.bin` encrypted documents; `vault.key.dpapi` the DPAPI-wrapped key.
- Sensitive profile fields, claim notes, document metadata and prefill packages are encrypted columns.
- Extracted document text is redacted of identifiers before it enters the FTS index (`vault.index_text=false` disables indexing).

## Events

Transactional outbox (`events` table) → broadcast after commit → SSE (`/api/v1/events`), `events.poll`, `jobs watch`, `discover watch`.

## Deviations from the MDR's suggested layout

The MDR suggests separate `matching`, `claims`, `notifications`, `jobs`, `agents` crates. They live as modules inside `core` because they share one transaction and one registry; the public surface is identical.
