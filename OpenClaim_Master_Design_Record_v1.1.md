# OPENCLAIM — MASTER DESIGN RECORD

**Version:** 1.1.0  
**Date:** 2026-10-19  
**Status:** Build-ready architecture and functional specification  
**Category:** Free/open-source consumer settlement discovery and claims assistant  
**License:** MIT for original project code  
**Primary stack:** Rust backend + Angular frontend + first-class Rust CLI + optional MCP tool server  
**Deployment:** Local-first Windows desktop, independent headless daemon/CLI; optional self-hosted web deployment  
**Cost policy:** Every OpenClaim component costs $0 and has an open-source implementation. No subscriptions, paid APIs, built-in AI models, hosted inference, inference credits, per-claim fees, commissions, ads, donation-gated features, or artificial caps. No AI integration is needed to use OpenClaim. Users supply their own hardware and internet connection. External agents are separate software and are not supplied or paid for by OpenClaim; the project imposes no paid agent/service requirement. An official claim process may independently impose requirements that OpenClaim does not control.

---

## 1. Vision

Build a genuinely free alternative to ClaimHunter and other consumer settlement finder products. OpenClaim discovers opportunities, cites authoritative sources, assesses **possible** eligibility, organizes evidence, assists with filling official forms, tracks deadlines/submissions/results, and finds free unclaimed-property searches. It never represents a predicted match as confirmed legal eligibility or promises a payout.

The product is not a claims administrator, financial intermediary, or legal adviser. Claim money goes directly from the official administrator or agency to the claimant. OpenClaim never takes custody of funds.

### Product principles

1. **Free means free:** No paywalls, cloud billing, hidden provider keys, revenue capture, or dependence on a commercial service.
2. **Local-first:** Profiles, claims, proof, and analysis remain on the user's PC by default.
3. **Evidence before automation:** Every opportunity has an official or clearly identified original source and a last-verified timestamp.
4. **No false claims:** Users attest to actual facts and explicitly approve filing; the app never invents purchases, dates, receipts, identities, or loss amounts.
5. **Accessible simplicity:** Clear wording, focused screens, keyboard support, screen readers, reduced motion, saved progress.
6. **Graceful offline mode:** Previously fetched data and local claim records remain available offline.
7. **No integrated AI:** OpenClaim ships with **zero local or hosted language models**, inference runtimes, AI API keys, or model downloads. External agents optionally control the complete application through the advanced Rust CLI and MCP adapter.
8. **No automation arms race:** Respect robots rules, source terms, rate limits, and CAPTCHAs; use human-guided handoff where automated access is unavailable.
9. **Agent-first parity:** CLI and GUI call the same domain services; headless operation needs no browser, frontend, or language model.
10. **Permissioned autonomy:** Externally controlled agents may research and organize permitted tasks, but identity-sensitive actions and official submission require explicit user involvement.

---

## 2. Primary jobs to be done

| Job | Expected result |
|---|---|
| Discover settlements | Searchable, deduplicated catalog of open, upcoming, closed, and pending opportunities |
| Understand eligibility | Plain-English explanation of class definition, location, date range, required proof, and exclusion conditions |
| Match history | Explainable suggested matches from locally stored user facts and evidence |
| Claim with confidence | Step-by-step checklist, link to original filing site, user-approved data prefill where appropriate |
| Track progress | Status, administrator reference, dates, proof, reminders, payouts manually confirmed or imported |
| Search unclaimed property | State-by-state and authorized federal directory with free official lookup links |
| Keep proof organized | Locally indexed receipts, notices, statements, PDF and image files |
| Understand changes | Alert when deadline, eligibility, claim form, or administrator notice changes |
| Let external agents operate OpenClaim | Discoverable Rust CLI and MCP tools, JSON/NDJSON output, jobs, dry runs, scoped credentials, approvals, and auditable actions |
| Work without the GUI | Start the headless Rust daemon, run discovery and reminders, and reconnect to the same SQLite-backed state later |

---

## 3. Strict free-only dependency and integration policy

**Allowed:** Open-source libraries, freely accessible government information, public RSS/Atom feeds, publicly readable official settlement/administrator pages where access is permitted, free downloadable public datasets, user-exported email files, and user-provided documents.

**Disallowed:** Paid court-record retrieval (e.g. billable PACER usage), proprietary settlement datasets requiring subscriptions, any bundled local/hosted LLM inference or AI-provider SDK, paid OCR, paid captcha-solving services, commercial proxy networks, paid push gateways, analytics SDKs with data resale, payment processors, and any hosted API for which the base experience depends on a paid or trial quota. The CLI/MCP interfaces are transport/protocol surfaces only, never model clients.

**Provider gate:** Before any source integration is enabled, the connector manifests `price=free`, `auth_requirement`, `robots_policy`, `terms_review_date`, `rate_limit`, and `automation_allowed`. If access becomes paid, the connector is disabled automatically and its previously verified data is retained with a freshness warning. Never auto-sign up for paid services or incur charges.

**No misleading universality:** Many government state databases provide interactive searches but not free public bulk-query APIs. OpenClaim must supply official links and human-assisted steps where a compliant automated search is unavailable. It must not claim to query every source automatically without proof of capability.

---

## 4. Source registry and acquisition strategy

### Tier A: Authoritative public sources

- **FTC active refund programs:** https://www.ftc.gov/enforcement/refunds
- **SEC Fair Funds and distributions:** https://www.sec.gov/enforcement-litigation/distributions-harmed-investors
- **State unclaimed property directories via NAUPA:** https://unclaimed.org/search/
- **NAUPA-endorsed multi-state search gateway:** https://www.missingmoney.com/ (search done on official site unless approved integration exists)
- **USAGov unclaimed money guidance:** https://www.usa.gov/unclaimed-money
- **U.S. DOJ public press releases and consumer notices:** https://www.justice.gov/ (discovery lead; not proof that a claim is open)
- **Individual official settlement administrator sites:** only after cross-checking their authenticity against court notices, agency postings, or published administrator contact details.

### Tier B: Discovery leads, not filing authority

- Public news and court-announcement feeds; publisher feeds that permit access.
- Community-submitted settlement tips with URLs.
- User-imported settlement emails, postcards, notices, and letters.
- Any third-party summary requires verification against a primary official source before the opportunity is marked `verified`.

### Free-data limitations

- Do not silently access paywalled court data or imply that all dockets are free.
- Government programs may redirect, be temporarily offline, or use CAPTCHAs; provide official manual lookup links.
- A settlement can have no claim form, no remaining deadline, or automatic distributions; normalize these possibilities rather than assuming a submission is available.
- Stated payout ranges are not guarantees. Display them only when supported by source text, alongside conditions and caveats.

### Connector lifecycle

`discover -> fetch -> parse -> normalize -> dedupe -> verify -> publish -> refresh -> expire/archive`

Each connector supports: stable source ID, origin URL, allowed access mode, freshness TTL, conditional HTTP fetch/ETag, parsed evidence spans, failure classification, backoff, source health dashboard, and a kill switch. Dedupe by normalized case title + court/docket identifier + settlement administrator + relevant dates; never merge solely on company name.

---

## 5. Functional modules

### 5.1 Settlement discovery

- Unified list/search by name, business, product, state, settlement type, administrator, period, proof requirements, and deadline.
- Filter: open / upcoming / automatic distribution / past deadline / awaiting administrator verification.
- Chronological discovery timeline, new opportunity alerts, recently updated notices.
- Cards show case title, factual class summary, jurisdictions, source, claim status, deadline, potential benefit **when confirmed**, last verified, and original links.
- Bookmark, hide, report inaccurate, and compare opportunities.
- No fabricated real-time data: stale or unverified results are visibly flagged.

### 5.2 Eligibility rules engine

Represent class membership using explicit criteria: geography, residence during a specified interval, purchase/service relationship, dates, eligibility exclusions, documentation needs, subclass conditions, and one-claim-per-person/household rules. Support `yes`, `no`, `unknown` for every condition.

Output states:
- `potential_match`: facts support key conditions, but legal eligibility is not established;
- `needs_more_information`: key evidence missing;
- `unlikely`: one or more hard conditions conflict with user facts;
- `not_applicable`: user confirms they were not part of the class;
- `manual_review`: ambiguous, contradictory, or complex terms.

Every result has a *why matched* explanation, the quoted/paraphrased rule with source link, missing facts, and user confirmation controls. Re-evaluate when settlement terms or profile facts change.

### 5.3 User and household profiles

- Multiple profiles stored locally; every adult's explicit authorization required to handle their submission.
- Profiles include aliases/previous names, state residency timeline, employers, purchase categories, memberships, communication preferences, and saved shipping/mailing addresses where desired.
- Optional proof snapshots, receipt dates, vehicle ownership periods, subscriptions, transaction descriptions.
- No SSN or bank account storage by default; sensitive facts entered only when actually necessary for an official process.
- Do not reuse another household member's eligibility or permission.

### 5.4 Local evidence vault

- Drag-and-drop import of PDFs, images, CSV, EML, MBOX, and plain text.
- PDF text extraction and optional Tesseract OCR, both on-device.
- Document classification and extraction using deterministic parsers, local PDF text extraction, Tesseract OCR, and human confirmation. If an external agent proposes extracted facts through the CLI, persist them as **untrusted suggestions** linked to source spans, not as confirmed user facts.
- Evidence links always point to an original local file plus extract/position and require human validation before inclusion in a claim.
- Local content search using SQLite FTS5. Dedup by SHA-256. Explicit user-controlled retention/deletion.
- Optional user-provided IMAP connection only when a free account supports it and only with explicit permissions; the default is manual EML/MBOX import. Never require Gmail/Outlook OAuth or cloud ingestion.

### 5.5 Claim filing assistant

1. Show exact eligibility checklist and original court/administrator evidence.
2. Warn about missing proof, excluded dates, duplicate-submission restrictions, and official deadlines.
3. Open the verified official claim URL.
4. Optionally fill **non-sensitive** fields through a consented local browser extension on user-approved domains, after a field-by-field preview.
5. Leave CAPTCHA, identity verification, payment destination details, electronic signature, and `Submit` to the user.
6. Capture a local confirmation identifier or user-entered receipt only after actual submission evidence exists.
7. Track status with manual updates or published administrator notifications.

The app must not silently submit claims, make statements on behalf of users, generate fake proof, or bypass administrator defenses.

### 5.6 Unclaimed property assistant

- Directory of 50 states + DC and other official searchable programs with accurate links and instructions.
- Save states where user has lived/worked; build a private checklist rather than pretending a nationwide search has completed.
- Guide the user to NAUPA/MissingMoney and each relevant official site.
- Store user-entered potential matches and status, not unauthorized bulk scraping of individual names.
- Support heirs' claims with a separate user-guided checklist; make authority/identity requirements explicit.

### 5.7 Deadlines, notifications, and tracking

- Local reminder scheduler: default 30, 14, 7, 3, and 1 day before verified deadline; configurable and timezone-aware.
- Desktop/browser notifications while app service is running; optional local calendar export via ICS.
- Claim state machine: `discovered -> reviewing -> possibly_eligible -> proof_needed -> ready -> opened_official_form -> submitted -> pending -> approved|denied|paid|withdrawn|expired`.
- No inferred approval, submission, or payout. Capture user confirmation and official evidence for status changes.
- Ledger of recovered amounts, currency, dates and reporting notes; no commissions.

### 5.8 Source verification and user corrections

- Every record exposes provenance, source type, retrieved time, source hash, changed-field diff, and verification state.
- Automatic verification requires a primary source confirming class terms and claim conditions.
- Changes to deadlines/eligibility trigger re-review and an explicit alert.
- Community tips are quarantined until a primary source corroborates them.
- Verified official domains are not automatically considered safe for all content; check redirects and link destinations every time.

---

## 6. Technical architecture

```text
  External agent (any harness; no OpenClaim-managed model)
                 |                 \
      command execution             MCP stdio tools
                 |                   |
                 +------> openclaim CLI <-------+
                               |
                   authenticated local IPC
                               |
┌──────────────────────────────▼──────────────────────────────────┐
│ Rust headless daemon (single source of truth)                  │
│ Discovery | Verified sources | Rules | Evidence | Claims       │
│ Jobs | Scheduler | Agent approvals | Audit | Data export        │
├──────────────────────────────┬──────────────────────────────────┤
│ SQLite WAL + FTS5            │ Local encrypted evidence vault   │
└──────────────────────────────┴──────────────────────────────────┘
                    ▲                     ▲
           REST/SSE or local IPC          |
                    |                     |
┌───────────────────┴─────────────────────┴──────────────────────┐
│ Angular GUI — optional client; may be closed at any time       │
│ Dashboard | Claim review | Vault | Agent activity | Approvals   │
└────────────────────────────────────────────────────────────────┘

Only permitted free public sources are fetched by the Rust daemon.
There is no embedded LLM runtime, cloud inference client, or Ollama.
```

**Backend:** Rust stable, Tokio, Axum, Serde, SQLx + SQLite, Reqwest, scraper, quick-xml/feed-rs for structured feeds, tracing, tower-http, clap v4, schemars, time, uuid, argon2, chacha20poly1305, sha2, keyring/OS secrets where appropriate. Pin via lockfile and audit dependencies.

**Frontend:** Angular current stable, TypeScript strict, standalone components, signals for local UI state, RxJS where streams are useful, Angular CDK for accessible overlays, CSS variables and vanilla CSS/SCSS, no paid component library.

**Packaging:** Separate `openclaim.exe` CLI and `openclaimd.exe` daemon, with a bundled Angular GUI accessible through localhost; optional Tauri v2 desktop wrapper. The CLI/daemon install and run without Angular, Node.js, a browser, a GPU, or any language model. Offline PWA cache is optional and must not cache personal claim evidence without explicit protection.

**Search:** SQLite FTS5 + well-defined filters, deterministic ranking, explainable facets. No embeddings or AI models are bundled or required.

**Background processing:** Tokio scheduler persisted to SQLite. Bounded concurrency, typed jobs, retry-after support, crash-safe resumption, connector-specific circuit breakers.

**Communication:** Rust CLI and MCP adapter talk to `openclaimd` over authenticated local IPC (Windows named pipe with owner ACL; Unix domain socket with 0600 permissions). Angular talks to the daemon through a localhost REST API and SSE. Bind to `127.0.0.1` by default; protect against DNS rebinding/CSRF, validate Origin, use a random session secret and explicit CORS allowlist. Exposing on LAN is a separate opt-in hardened mode.

**Agent interface:** No `AiProvider`, no inference integration, and no model configuration. `openclaim` provides human-readable and schema-versioned JSON/NDJSON commands. `openclaim mcp serve` wraps permissioned Rust services as locally spawned MCP stdio tools. External agents may interpret retrieved material and propose changes, but the Rust service validates schemas, source citations, approvals, rule versions, and side-effect policy before accepting mutations. External agent text is not authoritative legal evidence.

---

## 7. Suggested monorepo

```text
openclaim/
├── apps/
│   ├── desktop/              # optional Tauri v2 shell
│   ├── web/                  # Angular UI
│   ├── cli/                  # Rust command-line binary (openclaim)
│   └── browser-extension/    # optional consented form filling
├── crates/
│   ├── api/                  # Axum endpoints
│   ├── domain/               # types, rules, claim state machines
│   ├── db/                   # SQLite migrations and queries
│   ├── discovery/            # source registry, ingestion
│   ├── extract/              # PDF / document parsing
│   ├── matching/             # eligibility rules
│   ├── claims/               # claim workflows
│   ├── notifications/        # scheduler and alert drivers
│   ├── jobs/                 # persisted, resumable queue and workflows
│   ├── ipc/                  # secure local IPC transport
│   ├── cli-core/             # clap commands, output contracts, schemas
│   ├── mcp-adapter/          # optional stdio MCP server, no inference
│   ├── agents/               # principal scopes, approvals, event journal
│   └── security/             # cryptography, keys, trust policy
├── migrations/
├── source-manifests/         # signed/reviewed connector definitions
├── test-fixtures/           # public or synthetic examples only
├── docs/                    # architecture, privacy, source policies
├── Cargo.toml
├── Cargo.lock
├── package.json
├── LICENSE
└── README.md
```

---

## 8. Data model

| Table | Core fields |
|---|---|
| `opportunities` | id, title, type, administrator, case_number, official_url, status, claim_deadline, class_period_start/end, benefit_description, verified_at |
| `opportunity_rules` | opportunity_id, criteria_ast_json, original_text_excerpt, evidence_id, version |
| `sources` | id, URL, authority_tier, trust_status, free_only, robots/terms metadata, next_fetch_at, last_ok_at |
| `source_snapshots` | source_id, content_hash, fetched_at, change_summary, parser_version |
| `evidence_links` | opportunity_id, source_snapshot_id, precise_locator, assertion_type |
| `people` | id, display_name, encrypted_optional_details, consent_flags |
| `residence_periods` | person_id, jurisdiction, start/end, provenance |
| `consumer_facts` | person_id, type, merchant, product, event_dates, proof_id, asserted_by_user |
| `documents` | id, local_path, checksum, type, encrypted_metadata, imported_at |
| `matches` | person_id, opportunity_id, assessment, reasons_json, rules_version, evaluated_at |
| `claims` | person_id, opportunity_id, status, submitted_at, external_confirmation_id, manual_paid_amount |
| `claim_events` | claim_id, event_type, who_recorded, timestamp, evidence_id, metadata |
| `alerts` | subject_id, due_at, channel, delivered_at, disposition |
| `audit_log` | actor, action, timestamp, redacted_subject, integrity_hash |
| `agent_principals` | id, display_name, executor_fingerprint, scopes, owner_id, created_at, revoked_at |
| `agent_sessions` | id, principal_id, started_at, expires_at, last_seen, trace_id |
| `agent_approvals` | id, principal_id, action_digest, scope, preview_json, expires_at, decision, decided_by |
| `command_runs` | id, actor_id, command, args_redacted_json, request_id, result, duration_ms, created_at |
| `jobs` | id, kind, state, progress, owner_actor_id, next_run_at, retry_policy, cursor, error_code |
| `job_events` | id, job_id, event_type, payload_redacted_json, at |
| `proposed_facts` | id, person_id, document_id, proposal_json, evidence_locator, agent_id, validation_state |
| `idempotency_keys` | actor_id, command_scope, key, request_hash, response_ref, expires_at |

Use schema migrations and foreign keys, soft-delete only where needed, and a hard-delete route for user information. Monetary amounts use integer minor units/decimal types, never binary floating-point.

### Representative opportunity JSON

```json
{
  "id": "opportunity-uuid",
  "title": "Example Settlement (illustrative only)",
  "status": "verification_pending",
  "official_url": null,
  "claim_deadline": null,
  "eligibility": {
    "geography": [],
    "qualifying_events": [],
    "class_period": null,
    "proof_required": "unknown"
  },
  "benefit": {
    "amount_known": false,
    "description": null
  },
  "source_evidence": [],
  "last_verified_at": null
}
```

Never insert a placeholder as a live settlement.

---

## 9. Matching algorithm

A deterministic rules AST evaluates facts against case-specific conditions. Prioritize trustworthy, auditable answers over a magic single score.

```text
for opportunity in current_verified_opportunities:
    criteria = parse_verified_rules(opportunity)
    assessments = evaluate_three_valued_logic(criteria, local_profile_facts)
    if any_required_condition_is_false:
        state = "unlikely"
    elif unresolved_required_conditions:
        state = "needs_more_information"
    elif all_known_conditions_appear_supported:
        state = "potential_match"
    else:
        state = "manual_review"
    persist(state, each_reason_with_citation, rules_version)
```

Optional UI prioritization may rank by verified deadline proximity, match completeness, age of source verification, and required effort. Any estimated dollar value must never override uncertainty or produce a guaranteed-eligibility badge. The precise class definition is authoritative, not the AI extraction.

---

## 10. Interface and interaction design

**Visual language:** Sophisticated dark UI; near-black surfaces, restrained cyan/pink accents, high-contrast typography; no default yellow-heavy alerts, gradients, skeuomorphic financial imagery, or dashboard clutter. Full light theme and WCAG AA accessible contrast.

**Navigation:** Overview • Discover • My Matches • Claims • Property Search • Evidence Vault • Sources • Agent Activity • Approvals • Settings.

**Main dashboard:**

```text
┌─────────────────────────────────────────────────────────────────────┐
│ OPENCLAIM                Search settlements…        Sources: healthy │
├───────────────┬─────────────────────────────────────────────────────┤
│ Overview      │ Potential matches  | Needs review | Upcoming dates │
│ Discover      ├─────────────────────────────────────────────────────┤
│ My Matches    │ MATCHED OPPORTUNITIES                               │
│ Claims        │ [Settlement] [Why it matches] [Official source]     │
│ Property      │ [Settlement] [Missing evidence] [Save / Review]    │
│ Vault         ├─────────────────────────────────────────────────────┤
│ Sources       │ NEXT DEADLINES              RECENT VERIFIED UPDATES │
│ Settings      │ ...                         ...                     │
└───────────────┴─────────────────────────────────────────────────────┘
```

**Claim detail:** Summary → Eligibility → Supporting sources → Required evidence → Official filing → Tracking. Keep persistent "View original source" and "Mark uncertain" actions.

**Accessible use:** Keyboard-first flow, one decision per step, clear focus indicators, compact/expanded reading modes, reduced motion, adjustable font size, progress preservation, plain-language glossary, color-independent statuses, never time-limit local forms.

**Trust displays:** Every opportunity shows `Primary source`, `Verified on`, `Claim deadline`, and `Eligibility still requires review` wherever applicable. A visible **Agent Activity** screen shows current CLI/MCP jobs, exact proposed data changes, requesting agent, needed approval, audit history, and one-click revoke/kill controls. Closing the GUI never cancels daemon jobs.

---

## 11. API contract (v1)

| Method | Endpoint | Purpose |
|---|---|---|
| GET | `/api/v1/opportunities` | paginated search/filter |
| GET | `/api/v1/opportunities/{id}` | full details + sourced rules |
| GET | `/api/v1/opportunities/{id}/provenance` | provenance snapshots and links |
| POST | `/api/v1/matches/evaluate` | evaluate profile against verified rules |
| GET | `/api/v1/people` | local claimant profiles |
| POST | `/api/v1/claims` | save user-approved claim tracking record |
| PATCH | `/api/v1/claims/{id}` | track changes with audit event |
| POST | `/api/v1/documents/import` | import user-selected local document |
| GET | `/api/v1/sources/health` | source freshness and errors |
| POST | `/api/v1/sources/refresh` | manual refresh permitted connectors |
| GET | `/api/v1/alerts` | due and upcoming reminders |
| GET | `/api/v1/events` | SSE events (progress/changes) |
| POST | `/api/v1/exports` | export user's records, Markdown/CSV/JSON |
| DELETE | `/api/v1/user-data` | user-initiated local wipe with confirmation |
| GET | `/api/v1/jobs` | paginated job state and provenance |
| POST | `/api/v1/jobs/{id}/cancel` | permission-checked job cancellation |
| GET | `/api/v1/agents/sessions` | active local CLI/MCP agent sessions |
| GET | `/api/v1/agents/approvals` | pending approval requests |
| POST | `/api/v1/agents/approvals/{id}/decision` | explicit human approve/deny, with digest binding |
| POST | `/api/v1/agents/revoke/{id}` | revoke an agent principal |

Local API contracts via OpenAPI and automatically generated TypeScript client; use request and response validation, idempotency for updates, pagination, structured errors, and versioning.

---

## 12. Security, privacy, and trust

- **No mandatory accounts, telemetry, ad tracking, cloud analytics, or automatic background upload of personal documents.**
- All data at rest remains local. Encrypt sensitive profile fields/files with authenticated encryption; derive vault keys from a passphrase or OS protected secret. Explain recovery limitations.
- Sanitize imported markup, disable scripts in document previews, filter PDF/HTML threats, cap input sizes, block directory traversal, protect against archive bombs.
- Defend network fetching against SSRF, IP-literal/private-IP/localhost redirects, redirects to non-allowlisted hosts, DNS changes, TLS downgrade, and dangerous URL schemes.
- Treat source content as **untrusted data**; never let scraped text or documents issue instructions to external agents through OpenClaim tool responses. Strip active content, annotate provenance, and clearly separate data from CLI command instructions.
- Browser extension must have per-site opt-in, minimal permissions, field diff/undo, no password/payment/SSN autofill by default, and never click Submit automatically.
- No CAPTCHA circumvention or bot-evasion features.
- Never transmit local personal documents or sensitive government identifiers to an agent automatically. Agents only see the minimum scoped fields requested and authorized; full local files require a deliberate per-document read grant. There is no built-in AI provider or network inference.
- Support offline export/import of data and provenance. Prefer reproducible builds, signed release artifacts, dependency checks, SBOM, vulnerability audits and documented security reporting.
- Keep an audit trail for user actions and source updates without persisting plaintext secrets.

---

## 13. Background jobs

- **Source fetch:** daily only where permitted; conditional fetch, per-domain rate limits, exponential backoff.
- **Verification:** on every newly discovered case and changed source page; hold `verification_pending` until sufficient evidence.
- **Matching:** after rules, source, or locally confirmed user facts change.
- **Alerts:** local deadline sweep; no misleading notification if deadline not verified.
- **Integrity:** weekly local database checks and recoverable backup prompts.
- **Health:** report disabled/broken sources explicitly; source outages must never silently appear as “no settlements.”
- **Headless continuity:** daemon owns persistent schedules and jobs; command exit, terminal disconnect, or Angular window closure does not cancel durable jobs.
- **Approval timeouts:** expire unused human approval requests; paused mutations do not auto-approve on timeout.

---

## 14. Testing and acceptance criteria

### Required tests

1. Fixture-based parser tests for HTML, PDF, RSS, invalid pages, changed layouts, and anti-bot blocks.
2. Rule engine tests for qualifying/excluding jurisdictions and intervals, missing data, multi-person records, changed eligibility rules.
3. Dedup tests for same settlement across multiple publishers without merging different settlements involving the same company.
4. Security tests for malicious source URLs, prompt injection in agent-readable tool content, crafted PDFs, CORS, local IPC ACLs, secret redaction, identity leakage.
5. Browser extension tests showing that no form is submitted by the extension and user can undo a prefill.
6. E2E Angular tests with Playwright or an open-source equivalent; Rust unit/integration/contract tests; accessibility checks.
7. Dependency audit and offline installation/boot tests on Windows 11.
8. CLI golden-output fixtures for text/JSON/NDJSON, deterministic exit codes, pagination, idempotency, output schemas, and `--dry-run` guarantees.
9. MCP stdio protocol tests, tool resource limits, permissions, approval expirations, and hostile tool arguments.
10. Simultaneous Angular/CLI/agent edits, SQLite transaction conflicts, queue restart recovery, and daemon-only headless tests.

### Non-negotiable release gates

- No user account, card, paid API key, or subscription needed to discover and track opportunities.
- No network transmission of locally imported personal documents except when the user independently submits materials to an official claims website.
- All live opportunities have source provenance; unverified ones are clearly labeled and excluded from verified-only discovery.
- Each eligibility suggestion explains the rules used and unresolved conditions.
- No automatic official-site submission.
- No charged provider calls or premium switches anywhere in configuration or code.
- Backup/export/delete flows work; offline local history remains usable.
- A blocked source produces a clear failure/official-link fallback rather than bypassing its restrictions.
- Rust CLI parity for every supported non-visual workflow, with a machine-discoverable JSON schema and documented subcommands.
- Entire install and deterministic core operate with zero LLMs, no inference runtime, no GPU, and no AI-provider connection.
- UI can be closed while CLI jobs run, and the UI reopens displaying the real persisted state.
- Agent sessions are least-privilege, revocable, fully audited and never submit a claim on their own.

### Performance targets (engineering goals, not measured claims)

- Cold start on typical consumer Windows 11 SSD: <= 5 seconds excluding initial database migrations.
- Local query over 25k indexed opportunities: median < 250 ms.
- Typical Angular view navigation: <= 150 ms after assets are loaded.
- Local matching without AI: <= 100 ms per opportunity/profile for standard rule sets.
- Memory budget: track the headless daemon and CLI separately; never load an AI inference process.

---

## 15. Delivery roadmap

### Phase 0 — Foundation

Rust workspace, Angular UI, `openclaim` CLI, `openclaimd` headless service, SQLite migrations, authenticated local IPC, localhost security boundary, CI, lint, formatter, tests, installer skeleton and source registry. No mandatory user login.

### Phase 1 — Real discovery MVP

FTC and SEC read-only connectors, verification/freshness stamps, deduplication, discovery cards, official links, source audit panel, bookmarks and manual claim tracking. Import/export JSON and CSV.

### Phase 2 — Eligibility and personal organization

Explicit rules engine, profile timeline, local evidence vault, PDF text/OCR, matching explanations, deadlines and local notifications, official unclaimed property directory.

### Phase 3 — Filing assistance

Official-site guided forms, optional explicitly authorized browser extension, prefill preview, confirmation capture after user submission, per-claim status and payout logging.

### Phase 4 — Advanced agent control and refinement

Full CLI domain coverage, self-describing schemas, optional MCP stdio bridge, agent-scoped grants/approvals, resumable multi-step jobs, source change-diff engine, multiple claimants with individual authorization, and safe external-agent evidence proposals. No local language model or inference runtime.

### Phase 5 — Hardened release

CLI contract stabilization, MCP compatibility fixtures, concurrency/IPC validation, and installer signing where feasible without imposing paid service dependencies, reproducible packaging, tests/audit, accessibility pass, manuals, issue templates, example source connectors, and contribution guide. If an optional release-signing route costs money, use a fully free documented distribution process instead.

---

## 16. Advanced CLI system — mandatory first-class product surface

The primary automation interface is a native Rust executable named **`openclaim`**. It is not a collection of shell wrappers around web endpoints and it is not a chatbot. It uses the same typed Rust application services as Angular. Every substantive domain operation must be reachable through the CLI, including agent/session administration, source health, background jobs, evidence proposals, and exports. UI-only affordances (visual preview, CAPTCHA, human official-site submission) expose CLI *handoff* operations rather than automated equivalents.

### 16.1 Nonnegotiable interface contracts

- No model runtime, inference API client, or model dependency in any binary or build profile.
- Run fully headless on Windows 11 using PowerShell, CMD, external coding harnesses, or other process runners.
- `openclaimd` is the sole SQLite writer; CLI and Angular submit commands to its shared transactional domain services. SQLite WAL is enabled, but do not permit competing independent writer processes.
- GUI closure, CLI process exit, and agent disconnect do not stop daemon jobs or reminders.
- Human and machine output are separated: `--output table|text|json|ndjson`; `--output json` returns exactly one JSON object on stdout; `--output ndjson` streams one JSON event per line. Progress and diagnostics go to stderr in human mode.
- Machine-facing output is schema-versioned; avoid undocumented prose before or after JSON.
- All commands support `--help`, `--version`, and command tree discovery. Implement `openclaim help --output json` and `openclaim schema export`.
- Consistent global flags: `--profile`, `--output`, `--fields`, `--limit`, `--cursor`, `--timeout`, `--offline`, `--dry-run`, `--request-id`, `--idempotency-key`, and `--non-interactive` (where applicable).
- `--dry-run` computes a validation/side-effect preview and **never commits a mutation**, queues jobs, imports files or triggers an external request with side effects. Commands must report known read-only fetches separately.
- All expensive tasks return a durable `job_id`, current state, progress, and next action; provide `jobs watch --format ndjson`, `jobs wait`, `jobs cancel`, and `jobs retry`.
- Interactive commands never hang inside an agent execution: with `--non-interactive`, missing permission yields `APPROVAL_REQUIRED` and a machine-readable approval ID rather than a terminal prompt.
- Command line help, completion scripts (PowerShell/Bash/Zsh), manpages, and examples ship with the application.
- Profile data access is explicitly scoped; an agent is **not** entitled to read sensitive evidence or identities because it can execute the binary.
- Local-first: no external telemetry, login gateway, remote tool directory, AI service endpoint, or billing infrastructure.

### 16.2 CLI invocation and reliable execution

```text
External agent / terminal / workflow orchestrator
                |
                v
      openclaim <subcommand>
                |
          clap typed parser
                |
         schema validation
                |
  local IPC to authenticated daemon
                |
  scoped authorization + risk policy
                |
      Rust domain service (transaction)
                |
     SQLite + evidence vault + job queue
                |
          JSON result + audit record
```

- Rust `clap` derive for ergonomics and subcommands; `serde` and `schemars` for typed, published input/output contracts.
- Windows IPC uses a named pipe with owner-only ACL; Linux/macOS use protected Unix-domain sockets. Reuse the same permission policy for localhost REST and the MCP adapter.
- Single-owner daemon lock, robust auto-start (`openclaim daemon ensure`), and `openclaim daemon status` inspection. Daemon auto-start is explicit/configurable for restricted environments.
- Each request gets a trace ID, request ID, actor ID, causal job ID when applicable, and idempotency token for retried mutations.
- Read-only commands should support stable pagination, filter expressions, sorting, projection, source-time bounds, and snapshots to prevent paging across inconsistent refreshes.
- Provide atomic CLI batch transactions for **local metadata changes only**. Do not claim atomicity for network fetches or changes at third-party websites.

---

## 17. CLI command tree — minimum complete surface

This is the **normative target syntax** to implement, not a claim that a repository already supports the commands.

| Namespace | Required capabilities |
|---|---|
| `openclaim daemon` | `ensure`, `start`, `stop`, `restart`, `status`, `health`, `logs`, `doctor` |
| `openclaim discover` | `search`, `new`, `trending` (deterministic activity sort), `compare`, `watch`, `history` |
| `openclaim opportunities` | `list`, `show`, `rules`, `sources`, `changes`, `verify`, `bookmark`, `hide`, `export` |
| `openclaim sources` | `list`, `show`, `health`, `refresh`, `test`, `validate`, `register`, `disable`, `history` |
| `openclaim match` | `evaluate`, `explain`, `missing`, `refresh`, `compare`, `export` |
| `openclaim people` | `list`, `show`, `create`, `update`, `residences`, `history`, `consent`, `delete` |
| `openclaim facts` | `list`, `propose`, `validate`, `confirm`, `reject`, `link-proof`, `history` |
| `openclaim documents` | `import`, `list`, `show`, `extract`, `search`, `link`, `redact`, `export`, `delete` |
| `openclaim claims` | `list`, `show`, `init`, `checklist`, `prepare`, `handoff`, `record-submission`, `update`, `history`, `export` |
| `openclaim property` | `states`, `official-links`, `checklist`, `research`, `track`, `history` |
| `openclaim alerts` | `list`, `add`, `snooze`, `dismiss`, `due`, `export-ics` |
| `openclaim jobs` | `list`, `show`, `enqueue`, `watch`, `wait`, `cancel`, `retry`, `history` |
| `openclaim workflows` | `list`, `validate`, `plan`, `run`, `resume`, `status`, `cancel`, `export` |
| `openclaim agents` | `list`, `register`, `show`, `authorize`, `scopes`, `sessions`, `revoke`, `audit` |
| `openclaim approvals` | `list`, `show`, `request`, `approve`, `deny`, `expire` |
| `openclaim audit` | `list`, `show`, `export`, `verify` |
| `openclaim data` | `backup`, `restore`, `export`, `import`, `integrity`, `wipe` |
| `openclaim config` | `show`, `get`, `set`, `validate`, `reset` |
| `openclaim schema` | `export`, `commands`, `describe`, `version`, `validate` |
| `openclaim mcp` | `serve`, `inspect`, `list-tools`, `test` (agent interface, no model) |
| `openclaim completion` | `powershell`, `bash`, `zsh` |

### Common search/filter vocabulary

Search supports a canonical typed filter DSL or JSON query file, not arbitrary SQL. Fields include `state`, `status`, `deadline_after/before`, `verified_since`, `benefit_type`, `claim_method`, `proof_required`, `administrator`, `source_health`, `match_state`, and `only_verified`. Document full-text search must operate on local FTS5 with opt-in grants to agents.

For every namespace, ship a typed argument schema plus example output and error schema. An LLM agent should be able to discover correct usage without parsing an ASCII `--help` screen.

---

## 18. Agent-usable CLI examples and machine contracts

The examples below define proposed CLI behavior; they are **design targets** pending implementation.

### Discovery and rule review

```powershell
# Launch/verify the Rust service; no Angular window required
openclaim daemon ensure --output json

# Find only verified claims with deadlines in the future
openclaim discover search --status open --only-verified --sort deadline --output json --limit 25

# Retrieve detailed primary evidence and provenance
openclaim opportunities show <OPPORTUNITY_ID> --include sources,rules,provenance --output json

# Compute deterministic assessment for an authorized profile
openclaim match evaluate --person <PERSON_ID> --opportunity <OPPORTUNITY_ID> --output json
openclaim match explain --person <PERSON_ID> --opportunity <OPPORTUNITY_ID> --output json
```

### Evidence and claims

```powershell
# An agent needs an explicit evidence-read scope to receive document contents
openclaim documents import --file 'C:\Users\Me\Documents\receipt.pdf' --output json
openclaim documents extract --id <DOCUMENT_ID> --output json

# Agent-proposed facts enter an untrusted review queue; never auto-confirm
openclaim facts propose --person <PERSON_ID> --from-document <DOCUMENT_ID> --input-json proposed-fact.json --output json

# Build a claim package in local application storage, not at an official website
openclaim claims prepare --person <PERSON_ID> --opportunity <OPPORTUNITY_ID> --dry-run --output json
openclaim claims handoff --claim <CLAIM_ID> --output json

# Capture confirmation only after the user genuinely submits the claim
openclaim claims record-submission --claim <CLAIM_ID> --confirmation <OFFICIAL_ID> --user-confirmed --output json
```

### Long-running background work

```powershell
openclaim sources refresh --scope official --output json
openclaim jobs show <JOB_ID> --output json
openclaim jobs watch <JOB_ID> --output ndjson
openclaim jobs cancel <JOB_ID> --output json
openclaim daemon health --output json
```

### Agent security / control

```powershell
# Local setup with a human registering a named automation principal
openclaim agents register --name settlement-researcher --scope discover.read,sources.read,opportunities.read,jobs.read --output json

# Agent proposes a privileged action and gets an approval identifier
openclaim claims prepare --claim <CLAIM_ID> --non-interactive --output json

# Human can review what action will occur before deciding
openclaim approvals show <APPROVAL_ID> --output json
openclaim agents revoke <AGENT_ID> --output json

# Discover every command schema for external tools
openclaim schema export --format json --file openclaim-command-schema.json
openclaim mcp serve --transport stdio
```

### Stable JSON success envelope

```json
{
  "schema_version": "1.1",
  "ok": true,
  "request_id": "uuid",
  "trace_id": "uuid",
  "data": {
    "job_id": "uuid",
    "status": "queued"
  },
  "warnings": [],
  "pagination": null,
  "audit_event_id": "uuid"
}
```

### Stable JSON error envelope

```json
{
  "schema_version": "1.1",
  "ok": false,
  "request_id": "uuid",
  "error": {
    "code": "APPROVAL_REQUIRED",
    "message": "An explicitly authorized user must approve this action.",
    "retryable": false,
    "approval_id": "uuid",
    "needed_scopes": ["claims.prepare"]
  }
}
```

### NDJSON job stream

```jsonl
{"event":"job.queued","job_id":"uuid","sequence":1,"at":"2026-10-19T12:00:00Z"}
{"event":"job.progress","job_id":"uuid","sequence":2,"percent":40,"at":"2026-10-19T12:00:01Z"}
{"event":"job.completed","job_id":"uuid","sequence":3,"result_ref":"uuid","at":"2026-10-19T12:00:02Z"}
```

For `json`/`ndjson` mode, stdout **must never contain banners, progress bars, color codes, warnings as free text, or secrets**. `stderr` carries strictly diagnostic information. Large document blobs must be passed through bounded permissioned file handles or redacted chunk reads, never embedded unbounded into the success envelope.

---

## 19. MCP bridge and integration with external LLM agents

### 19.1 Design

`openclaim mcp serve --transport stdio` starts a **model-free** local Model Context Protocol (MCP) server. It exposes the same domain capabilities as the CLI with JSON-schema-based inputs/outputs, short descriptions, explicit effect classification, scoped principal identity, and structured errors. An external AI harness starts the process and decides when to call tools; OpenClaim does not choose, download, host, prompt, or bill a model.

Keep the bridge behind feature-gated open-source protocol dependencies and tested against the MCP specification version pinned in the project's compatibility manifest. Stdio is the preferred default; no public unauthenticated HTTP MCP listener.

### 19.2 Tool groups

| MCP surface | Semantics |
|---|---|
| `discover_opportunities` | Filter verified sources and catalog |
| `get_opportunity` | Return sourced case description and status |
| `get_opportunity_rules` | Return structured eligibility AST and source locators |
| `compare_opportunities` | Contrast two or more verified opportunities |
| `evaluate_match` | Run deterministic eligibility evaluation with reason codes |
| `list_document_metadata` | View authorized document catalog metadata |
| `read_document_excerpt` | Access bounded/redacted local evidence with read grant |
| `propose_fact` | Save a proposed, non-authoritative fact with source pointer |
| `get_claim_checklist` | Retrieve missing requirements and handoff next steps |
| `prepare_claim_draft` | Generate local user-review package, never submit externally |
| `get_claim_status` | Check local status and evidence for status |
| `refresh_source` | Start a compliant bounded source-refresh job |
| `get_job` / `watch_job` | Retrieve durable job status and progress |
| `list_alerts` | Check deadlines and changes |
| `request_approval` | Request a scoped human decision for a named action |
| `get_approval_status` | Poll human approval without auto-grant |
| `get_source_health` | Expose staleness, source failure and provenance |

MCP **resources** expose documentation, read-only source provenance, tool schemas, and permitted capability manifests. MCP **prompts**, if offered at all, are optional static guidance, not active model inference. No external model is a project dependency.

### 19.3 External compatibility

Compatible external agent runtimes can integrate by launching the CLI as a subprocess, issuing JSON commands, or registering the local MCP stdio server. Examples of possible host categories include coding agents, shell-based automation harnesses, and agent orchestration platforms. Integration must not require an OpenClaim account or a paid AI service. A third-party agent's licensing or inference costs are outside this application's control; the core app and agent-facing interfaces work without one.

Every tool result returned to an agent clearly delineates **untrusted public-source text**, cited source metadata, deterministic evaluation, agent-proposed data, and human-confirmed facts. No website content can add tools, grant scopes, or alter runtime instructions.

---

## 20. Multi-agent permissions, human approvals, and threat model

### 20.1 Capability policy

Principals are named local actors with `id`, owner, scope list, issue/expiration times, and revocation history. Registration and elevated grants require user action in the GUI or an authenticated interactive terminal, not agent self-granting. Protect IPC from arbitrary other-user processes; when untrusted agents execute under the same OS account, enforce explicit agent identities and scoped launch credentials rather than relying solely on OS identity.

Suggested capability families:

- `discover.read`, `opportunities.read`, `sources.read`, `match.evaluate`, `jobs.read`, `alerts.read`.
- `sources.refresh`, `documents.metadata`, `documents.read:document-id`, `people.read:profile-id`.
- `facts.propose`, `claims.draft`, `claims.track`, `workflows.run`, `alerts.write`.
- `people.modify`, `documents.import`, `documents.delete`, `data.export`, `data.wipe`, `agents.manage` — elevated, approval-gated by default.

There is no `claims.submit_official` agent capability. Completing CAPTCHA, identity verification, signatures, final filing, and any bank/payment destination changes remain outside autonomous tools.

### 20.2 Approval transaction lifecycle

`agent requests -> daemon validates -> computes normalized action digest -> stores proposed mutation + redacted preview -> human reviews in GUI or interactive CLI -> approves exact digest and scope -> daemon commits once -> audit records result`.

- Approvals expire and are bound to exact parameters, record versions, principal, and intended side effects.
- `--non-interactive` returns `APPROVAL_REQUIRED` rather than blocking.
- Source change, claim rule change, or altered parameters invalidates approval.
- Denial and timeouts are terminal until re-requested; they never default to approval.
- Emergency `agents revoke` cancels remaining queued privileged steps; executed actions remain visible in immutable audit history.
- Prevent confused deputy problems: a low-privilege actor cannot invoke a high-privilege workflow indirectly to acquire elevated actions.

### 20.3 Data access rules

Agent default is **read-only settlement research without personal identity access**. Reveal profile/evidence details only with explicit narrow grants. Redact SSNs, account numbers, payment details, full addresses, private contact info, and identifiers from logs and model-facing structured output by default. External agents can read user-approved excerpts but OpenClaim never silently uploads any evidence or bundles it into prompts. Support a one-click list of what was shared with each agent and an agent kill switch.

---

## 21. Durable workflows, scheduling, and concurrency

### 21.1 Workflow definitions

Provide a declarative versioned YAML/JSON workflow runner, interpreted by Rust only and limited to allowlisted domain operations. A workflow is a sequence/DAG of typed steps with explicit input/output bindings, branch predicates, deadlines, concurrency budgets, retry policies, and approval nodes. Do **not** allow arbitrary code execution from downloaded workflow files, unsandboxed templates, or source content.

Example conceptual workflow:

```yaml
schema_version: 1
name: daily-verified-discovery
permissions:
  - sources.refresh
  - opportunities.read
  - match.evaluate
  - alerts.write
steps:
  - id: refresh
    operation: sources.refresh
    with: {tier: official, respect_ttl: true}
  - id: search
    operation: opportunities.search
    after: [refresh]
    with: {only_verified: true, status: open}
  - id: create_alerts
    operation: alerts.from_deadlines
    after: [search]
    with: {deduplicate: true}
```

YAML above is illustrative; implement a concrete schema and validate it before execution. Human approval remains in force regardless of what a workflow requests.

### 21.2 Execution guarantees

- Durable, SQLite-backed queued/running/paused_for_approval/succeeded/failed/cancelled state machine.
- Explicit job leases, heartbeats, crash recovery, cancellation points, bounded retries, exponential backoff, progress checkpoints, and backpressure.
- Concurrency tokens per connector/domain and global ceiling. Respect robots, terms, and `Retry-After`.
- Exactly-once local *commit effects* where feasible via transactional outbox and idempotency keys; network effects are at-least-once attempts unless confirmed, and must be safe to retry or require human re-check.
- `openclaim workflows plan` previews requested scopes, sources, rate limits, likely outputs, and approval gates without running.
- `openclaim workflows run --detach` returns durable job ID; `--resume` continues from checkpoints.
- Agents can hand off tasks to other agents through shared **application** job queue/metadata, not direct unrestricted prompts or shared secrets.
- CLI, GUI, and MCP display the same canonical job state at the same time. Optimistic concurrency uses record ETags/version numbers; return `CONFLICT` and a refresh suggestion on stale writes.

---

## 22. CLI correctness, documentation, and quality gates

### 22.1 Exit codes and error handling

| Exit code | Symbolic class | Meaning |
|---|---|---|
| `0` | `OK` | Completed (or job successfully accepted) |
| `2` | `INVALID_ARGUMENT` | Syntax or input schema error |
| `3` | `NOT_FOUND` | No identified record / unsupported identifier |
| `4` | `PERMISSION_DENIED` | Actor lacks a scoped capability |
| `5` | `APPROVAL_REQUIRED` | Pending or required human approval |
| `6` | `CONFLICT` | Stale version/idempotency mismatch |
| `7` | `UNAVAILABLE` | Daemon/source unavailable, no unauthorized fallback |
| `8` | `TIMEOUT` | Command timeout; durable job may continue |
| `9` | `POLICY_BLOCKED` | Paid, disallowed, unsafe, or prohibited source/action |
| `10` | `PARTIAL_RESULT` | Source-limited result; explicit missing coverage |
| `11` | `INTERNAL` | Unexpected fault, trace ID required |

JSON envelopes include `retryable`, `retry_after`, `source_status`, `next_action`, and `details` as applicable. Exit codes are part of public compatibility guarantees. Never return exit code 0 for a failed agent mutation.

### 22.2 Quality and acceptance tests

1. Install on Windows 11 and run the CLI/daemon with **no browser, no Angular window, no local or cloud model, and no GPU**.
2. Verify every user-visible workflow (except inherent visual/manual tasks) has equivalent CLI and MCP capability.
3. Exercise each CLI command with valid/invalid args, offline mode, JSON/NDJSON output, `--help`, pagination, idempotency and `--dry-run`.
4. Assert stdout machine modes never emit extraneous data; compare contract fixtures against versioned JSON schemas.
5. Simulate compromised public HTML/PDF and demonstrate that its instructions cannot change agent scopes, CLI execution, or official source verification.
6. Prove that an agent with `discover.read` cannot read personal documents, register itself with elevated scopes, or execute claim-related modifications.
7. Validate human approval digest checks, expirations, stale record rejection, and interactive/noninteractive semantics.
8. Kill and restart CLI, Angular and daemon separately; verify durable jobs and provenance survive correctly, with no phantom submissions.
9. Run parallel CLI/MCP/GUI actions against one database and check no lost updates, double-imports or duplicated job effects.
10. Export a complete offline audit trail of what each agent queried, proposed, changed, and was allowed to read, with sensitive fields redacted.
11. Ensure no build graph dependency on Ollama, llama.cpp, ONNX model inference, hosted model SDKs, token billing, or model binaries.
12. Confirm all network source connectors and npm/crate dependencies are free to use for the shipped application; keep dependency license manifests current.

---

## 23. Developer commands (target workflow)

```bash
# Development goals after implementation/scaffolding
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all -- --check
npm ci
npm run test --workspace=apps/web
npm run build --workspace=apps/web
cargo build --release --bin openclaim --bin openclaimd
openclaim daemon ensure --output json
openclaim daemon health --output json
openclaim schema export --format json --file command-schema.json
openclaim mcp list-tools --output json
```

Define the actual workspace scripts and binaries during implementation. All commands are **specified targets**, not a claim of already-existing software. Build reproducibly and produce offline-friendly release artifacts and checksums; code signing must not become a paid prerequisite.

---

## 24. Explicit exclusions

- No subscriptions, donation-gated features, freemium limits, paid dashboards, pay-per-claim, cloud API charges, or checkout pages.
- **No Ollama, no llama.cpp, no local language models, no built-in cloud LLM access, no model inference adapters, and no optional model setting.**
- No built-in agent that independently chooses actions. Only external agents operating the documented CLI/MCP boundary.
- No mandatory external coding-agent purchase, paid API key, hosted tool gateway, or paid token quota.
- No court filings as a legal representative, legal advice, guaranteed settlements, or fake proof generation.
- No automatic monetary disbursement or bank-account custody.
- No automatic final official claim submission, identity attestation, signature, or CAPTCHA bypass by any agent.
- No indiscriminate mail crawling, bulk personal-data harvesting, or bypassing access restrictions.
- No false claim that a state or federal agency offers a free programmatic API when it only offers a web portal.
- No hidden dependence on Google Cloud, OpenAI, Anthropic, Twilio, Firebase paid tiers, Vercel paid hosting, or paid PACER access.

---

## 25. Definition of done

A Windows 11 user can install and run OpenClaim with the Angular GUI **or entirely from the advanced Rust CLI**. Both clients browse verified opportunities, run deterministic eligibility checks, inspect locally authorized evidence, guide the user to official claims, track statuses, find free government unclaimed-property search links, schedule background work and export/delete local data. The Rust daemon continues jobs while the GUI is closed; reopening the GUI reconstructs the exact persisted state. An external language-model agent can discover CLI/MCP tools, request scopes, execute permitted read/write workflows, subscribe to machine-readable progress, and request human approvals, without OpenClaim hosting, installing, invoking, or paying for a model. Final claims require real user review and authorization. No OpenClaim feature, dependency, or required source access is sold, gated, billed, or monetized.

---

## 26. Research references (reviewed 2026-10-18)

- ClaimHunter reference product/pricing: https://www.claimhunter.com/
- ClaimHunt consumer feature reference: https://claimhunt.app/
- FTC official refund programs: https://www.ftc.gov/enforcement/refunds
- SEC distributions: https://www.sec.gov/enforcement-litigation/distributions-harmed-investors
- NAUPA official free state searches: https://unclaimed.org/search/
- NAUPA on free state searches: https://unclaimed.org/can-i-really-search-for-free/
- USAGov government unclaimed money: https://www.usa.gov/unclaimed-money
- PACER pricing, deliberately excluded: https://pacer.uscourts.gov/pacer-pricing-how-fees-work

**Caveat:** No universal free bulk database contains all U.S. settlements or all personal unclaimed-property records. OpenClaim supports verifiable free sources and clear manual lookup fallbacks rather than unsupported coverage claims. External agent runtime/model costs, if a user independently chooses a paid one, are not OpenClaim costs or requirements.
