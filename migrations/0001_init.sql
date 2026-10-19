-- OpenClaim schema v1. Timestamps are RFC 3339 UTC text; dates are YYYY-MM-DD text.
-- Money is integer minor units. The daemon is the only writer.

CREATE TABLE meta (
    key   TEXT PRIMARY KEY,
    value TEXT NOT NULL
);

CREATE TABLE config (
    key        TEXT PRIMARY KEY,
    value_json TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

-- ---------------------------------------------------------------- sources
CREATE TABLE sources (
    id                 TEXT PRIMARY KEY,
    name               TEXT NOT NULL,
    url                TEXT NOT NULL,
    connector          TEXT NOT NULL,
    authority_tier     TEXT NOT NULL CHECK (authority_tier IN ('A','B','user')),
    trust_status       TEXT NOT NULL DEFAULT 'reviewed',
    free_only          INTEGER NOT NULL DEFAULT 1 CHECK (free_only = 1),
    manifest_json      TEXT NOT NULL,
    enabled            INTEGER NOT NULL DEFAULT 1,
    disabled_reason    TEXT,
    health             TEXT NOT NULL DEFAULT 'never_fetched',
    last_failure       TEXT,
    last_failure_detail TEXT,
    consecutive_failures INTEGER NOT NULL DEFAULT 0,
    etag               TEXT,
    last_modified      TEXT,
    next_fetch_at      TEXT,
    last_attempt_at    TEXT,
    last_ok_at         TEXT,
    created_at         TEXT NOT NULL,
    version            INTEGER NOT NULL DEFAULT 1
);

CREATE TABLE source_snapshots (
    id             TEXT PRIMARY KEY,
    source_id      TEXT NOT NULL REFERENCES sources(id),
    url            TEXT NOT NULL,
    content_hash   TEXT NOT NULL,
    fetched_at     TEXT NOT NULL,
    acquisition    TEXT NOT NULL CHECK (acquisition IN ('fetch','user_snapshot','fixture')),
    http_status    INTEGER,
    change_summary TEXT,
    parser_version TEXT NOT NULL,
    excerpt        TEXT
);
CREATE INDEX idx_snapshots_source ON source_snapshots(source_id, fetched_at);

-- ---------------------------------------------------------- opportunities
CREATE TABLE opportunities (
    id                 TEXT PRIMARY KEY,
    source_id          TEXT REFERENCES sources(id),
    external_key       TEXT,
    dedupe_key         TEXT NOT NULL,
    title              TEXT NOT NULL,
    opp_type           TEXT NOT NULL,
    administrator      TEXT,
    administrator_phone TEXT,
    case_number        TEXT,
    court              TEXT,
    official_url       TEXT,
    claim_url          TEXT,
    status             TEXT NOT NULL,
    verification_state TEXT NOT NULL,
    authority_tier     TEXT NOT NULL,
    claim_deadline     TEXT,
    deadline_verified  INTEGER NOT NULL DEFAULT 0,
    class_period_start TEXT,
    class_period_end   TEXT,
    jurisdictions_json TEXT NOT NULL DEFAULT '[]',
    summary            TEXT,
    benefit_json       TEXT NOT NULL DEFAULT '{"amount_known":false}',
    proof_required     TEXT NOT NULL DEFAULT 'unknown',
    claim_method       TEXT NOT NULL DEFAULT 'unknown',
    source_updated_on  TEXT,
    first_seen_at      TEXT NOT NULL,
    updated_at         TEXT NOT NULL,
    verified_at        TEXT,
    archived_at        TEXT,
    version            INTEGER NOT NULL DEFAULT 1,
    UNIQUE (source_id, external_key)
);
CREATE INDEX idx_opp_status ON opportunities(status, verification_state);
CREATE INDEX idx_opp_deadline ON opportunities(claim_deadline);
CREATE INDEX idx_opp_dedupe ON opportunities(dedupe_key);

CREATE VIRTUAL TABLE opportunities_fts USING fts5(
    opportunity_id UNINDEXED, title, administrator, summary, case_number,
    tokenize = 'porter unicode61'
);

CREATE TABLE opportunity_aliases (
    opportunity_id TEXT NOT NULL REFERENCES opportunities(id),
    source_id      TEXT NOT NULL,
    external_key   TEXT NOT NULL,
    title          TEXT NOT NULL,
    PRIMARY KEY (source_id, external_key)
);

CREATE TABLE opportunity_rules (
    opportunity_id    TEXT NOT NULL REFERENCES opportunities(id),
    version           INTEGER NOT NULL,
    criteria_ast_json TEXT NOT NULL,
    original_text_excerpt TEXT NOT NULL,
    evidence_id       TEXT,
    curated_by        TEXT NOT NULL,
    created_at        TEXT NOT NULL,
    PRIMARY KEY (opportunity_id, version)
);

CREATE TABLE evidence_links (
    id                 TEXT PRIMARY KEY,
    opportunity_id     TEXT NOT NULL REFERENCES opportunities(id),
    source_snapshot_id TEXT NOT NULL REFERENCES source_snapshots(id),
    precise_locator    TEXT NOT NULL,
    assertion_type     TEXT NOT NULL,
    excerpt            TEXT NOT NULL,
    created_at         TEXT NOT NULL
);
CREATE INDEX idx_evidence_opp ON evidence_links(opportunity_id);

CREATE TABLE opportunity_changes (
    id             TEXT PRIMARY KEY,
    opportunity_id TEXT NOT NULL REFERENCES opportunities(id),
    field          TEXT NOT NULL,
    old_value      TEXT,
    new_value      TEXT,
    snapshot_id    TEXT,
    detected_at    TEXT NOT NULL,
    reviewed_at    TEXT
);
CREATE INDEX idx_changes_opp ON opportunity_changes(opportunity_id, detected_at);

CREATE TABLE opportunity_marks (
    opportunity_id TEXT PRIMARY KEY REFERENCES opportunities(id),
    bookmarked     INTEGER NOT NULL DEFAULT 0,
    hidden         INTEGER NOT NULL DEFAULT 0,
    updated_at     TEXT NOT NULL
);

CREATE TABLE opportunity_reports (
    id             TEXT PRIMARY KEY,
    opportunity_id TEXT NOT NULL REFERENCES opportunities(id),
    reason         TEXT NOT NULL,
    reported_by    TEXT NOT NULL,
    created_at     TEXT NOT NULL,
    resolved_at    TEXT
);

-- Community/user tips are quarantined until a primary source corroborates them.
CREATE TABLE tips (
    id             TEXT PRIMARY KEY,
    url            TEXT NOT NULL,
    title          TEXT,
    note           TEXT,
    submitted_by   TEXT NOT NULL,
    state          TEXT NOT NULL DEFAULT 'quarantined' CHECK (state IN ('quarantined','corroborated','rejected')),
    opportunity_id TEXT REFERENCES opportunities(id),
    created_at     TEXT NOT NULL
);

-- ------------------------------------------------------ unclaimed property
CREATE TABLE property_programs (
    jurisdiction  TEXT PRIMARY KEY,
    name          TEXT NOT NULL,
    official_url  TEXT NOT NULL,
    kind          TEXT NOT NULL,
    snapshot_id   TEXT,
    automated_search INTEGER NOT NULL DEFAULT 0,
    instructions  TEXT NOT NULL,
    verified_at   TEXT NOT NULL
);

CREATE TABLE property_tracking (
    id            TEXT PRIMARY KEY,
    person_id     TEXT NOT NULL REFERENCES people(id) ON DELETE CASCADE,
    jurisdiction  TEXT NOT NULL,
    status        TEXT NOT NULL CHECK (status IN ('to_search','searched_no_match','possible_match','claim_started','claim_submitted','paid','not_mine')),
    heir_claim    INTEGER NOT NULL DEFAULT 0,
    notes_enc     TEXT,
    updated_at    TEXT NOT NULL,
    version       INTEGER NOT NULL DEFAULT 1,
    UNIQUE (person_id, jurisdiction)
);

CREATE TABLE property_events (
    id          TEXT PRIMARY KEY,
    tracking_id TEXT NOT NULL REFERENCES property_tracking(id) ON DELETE CASCADE,
    status      TEXT NOT NULL,
    recorded_by TEXT NOT NULL,
    at          TEXT NOT NULL
);

-- -------------------------------------------------------------- people
CREATE TABLE people (
    id                        TEXT PRIMARY KEY,
    display_name              TEXT NOT NULL,
    encrypted_optional_details TEXT,
    consent_flags_json        TEXT NOT NULL DEFAULT '{}',
    created_at                TEXT NOT NULL,
    updated_at                TEXT NOT NULL,
    version                   INTEGER NOT NULL DEFAULT 1
);

CREATE TABLE residence_periods (
    id           TEXT PRIMARY KEY,
    person_id    TEXT NOT NULL REFERENCES people(id) ON DELETE CASCADE,
    jurisdiction TEXT NOT NULL,
    start_date   TEXT,
    end_date     TEXT,
    provenance   TEXT NOT NULL,
    created_at   TEXT NOT NULL
);

CREATE TABLE consumer_facts (
    id              TEXT PRIMARY KEY,
    person_id       TEXT NOT NULL REFERENCES people(id) ON DELETE CASCADE,
    fact_type       TEXT NOT NULL,
    merchant        TEXT,
    product         TEXT,
    start_date      TEXT,
    end_date        TEXT,
    positive        INTEGER NOT NULL DEFAULT 1,
    proof_id        TEXT REFERENCES documents(id) ON DELETE SET NULL,
    asserted_by_user INTEGER NOT NULL CHECK (asserted_by_user = 1),
    source_proposal_id TEXT,
    created_at      TEXT NOT NULL
);

CREATE TABLE attestations (
    person_id      TEXT NOT NULL REFERENCES people(id) ON DELETE CASCADE,
    opportunity_id TEXT NOT NULL,
    key            TEXT NOT NULL,
    value          INTEGER NOT NULL,
    recorded_at    TEXT NOT NULL,
    PRIMARY KEY (person_id, opportunity_id, key)
);

CREATE TABLE class_exclusions (
    person_id      TEXT NOT NULL REFERENCES people(id) ON DELETE CASCADE,
    opportunity_id TEXT NOT NULL,
    recorded_at    TEXT NOT NULL,
    PRIMARY KEY (person_id, opportunity_id)
);

CREATE TABLE proposed_facts (
    id               TEXT PRIMARY KEY,
    person_id        TEXT NOT NULL REFERENCES people(id) ON DELETE CASCADE,
    document_id      TEXT REFERENCES documents(id) ON DELETE CASCADE,
    proposal_json    TEXT NOT NULL,
    evidence_locator TEXT,
    agent_id         TEXT NOT NULL,
    validation_state TEXT NOT NULL DEFAULT 'proposed' CHECK (validation_state IN ('proposed','confirmed','rejected')),
    decided_by       TEXT,
    decided_at       TEXT,
    created_at       TEXT NOT NULL
);

-- ----------------------------------------------------------- documents
CREATE TABLE documents (
    id                 TEXT PRIMARY KEY,
    vault_path         TEXT NOT NULL,
    checksum           TEXT NOT NULL UNIQUE,
    doc_type           TEXT NOT NULL,
    mime               TEXT NOT NULL,
    size_bytes         INTEGER NOT NULL,
    encrypted_metadata TEXT NOT NULL,
    extraction_state   TEXT NOT NULL DEFAULT 'pending',
    extraction_note    TEXT,
    person_id          TEXT REFERENCES people(id) ON DELETE SET NULL,
    imported_at        TEXT NOT NULL,
    version            INTEGER NOT NULL DEFAULT 1
);

-- Extracted text is redacted of identifiers before indexing.
CREATE VIRTUAL TABLE documents_fts USING fts5(document_id UNINDEXED, body, tokenize = 'porter unicode61');

CREATE TABLE document_links (
    document_id TEXT NOT NULL REFERENCES documents(id) ON DELETE CASCADE,
    target_kind TEXT NOT NULL CHECK (target_kind IN ('claim','fact','opportunity','person')),
    target_id   TEXT NOT NULL,
    locator     TEXT,
    validated   INTEGER NOT NULL DEFAULT 0,
    created_at  TEXT NOT NULL,
    PRIMARY KEY (document_id, target_kind, target_id)
);

-- --------------------------------------------------------------- matches
CREATE TABLE matches (
    person_id      TEXT NOT NULL REFERENCES people(id) ON DELETE CASCADE,
    opportunity_id TEXT NOT NULL REFERENCES opportunities(id),
    assessment     TEXT NOT NULL,
    reasons_json   TEXT NOT NULL,
    rules_version  INTEGER,
    stale          INTEGER NOT NULL DEFAULT 0,
    evaluated_at   TEXT NOT NULL,
    PRIMARY KEY (person_id, opportunity_id)
);

-- ---------------------------------------------------------------- claims
CREATE TABLE claims (
    id                       TEXT PRIMARY KEY,
    person_id                TEXT NOT NULL REFERENCES people(id) ON DELETE CASCADE,
    opportunity_id           TEXT NOT NULL REFERENCES opportunities(id),
    status                   TEXT NOT NULL,
    submitted_at             TEXT,
    external_confirmation_id TEXT,
    manual_paid_amount       INTEGER,
    currency                 TEXT,
    notes_enc                TEXT,
    created_at               TEXT NOT NULL,
    updated_at               TEXT NOT NULL,
    version                  INTEGER NOT NULL DEFAULT 1,
    UNIQUE (person_id, opportunity_id)
);

CREATE TABLE claim_events (
    id           TEXT PRIMARY KEY,
    claim_id     TEXT NOT NULL REFERENCES claims(id) ON DELETE CASCADE,
    event_type   TEXT NOT NULL,
    from_status  TEXT,
    to_status    TEXT,
    who_recorded TEXT NOT NULL,
    timestamp    TEXT NOT NULL,
    evidence_id  TEXT,
    metadata     TEXT NOT NULL DEFAULT '{}'
);

CREATE TABLE claim_packages (
    id             TEXT PRIMARY KEY,
    claim_id       TEXT NOT NULL REFERENCES claims(id) ON DELETE CASCADE,
    package_json   TEXT NOT NULL,
    prefill_enc    TEXT,
    allowed_domain TEXT,
    created_by     TEXT NOT NULL,
    created_at     TEXT NOT NULL,
    expires_at     TEXT NOT NULL
);

CREATE TABLE ledger (
    id           TEXT PRIMARY KEY,
    claim_id     TEXT REFERENCES claims(id) ON DELETE SET NULL,
    property_id  TEXT,
    amount_minor INTEGER NOT NULL,
    currency     TEXT NOT NULL,
    received_on  TEXT NOT NULL,
    note         TEXT,
    recorded_by  TEXT NOT NULL,
    created_at   TEXT NOT NULL
);

-- ---------------------------------------------------------------- alerts
CREATE TABLE alerts (
    id           TEXT PRIMARY KEY,
    subject_kind TEXT NOT NULL,
    subject_id   TEXT NOT NULL,
    kind         TEXT NOT NULL,
    title        TEXT NOT NULL,
    due_at       TEXT NOT NULL,
    channel      TEXT NOT NULL DEFAULT 'local',
    delivered_at TEXT,
    snoozed_until TEXT,
    disposition  TEXT NOT NULL DEFAULT 'pending' CHECK (disposition IN ('pending','delivered','dismissed','snoozed')),
    dedupe_key   TEXT UNIQUE,
    created_at   TEXT NOT NULL
);
CREATE INDEX idx_alerts_due ON alerts(disposition, due_at);

-- ------------------------------------------------- agents and approvals
CREATE TABLE owner (
    id               TEXT PRIMARY KEY CHECK (id = 'owner'),
    passphrase_hash  TEXT NOT NULL,
    created_at       TEXT NOT NULL,
    updated_at       TEXT NOT NULL
);

CREATE TABLE agent_principals (
    id                   TEXT PRIMARY KEY,
    display_name         TEXT NOT NULL UNIQUE,
    kind                 TEXT NOT NULL DEFAULT 'agent' CHECK (kind IN ('agent','extension')),
    token_hash           TEXT NOT NULL UNIQUE,
    executor_fingerprint TEXT,
    scopes_json          TEXT NOT NULL,
    owner_id             TEXT NOT NULL DEFAULT 'owner',
    allowed_origin       TEXT,
    created_at           TEXT NOT NULL,
    expires_at           TEXT,
    revoked_at           TEXT
);

CREATE TABLE agent_sessions (
    id           TEXT PRIMARY KEY,
    principal_id TEXT NOT NULL,
    transport    TEXT NOT NULL,
    started_at   TEXT NOT NULL,
    expires_at   TEXT,
    last_seen    TEXT NOT NULL,
    trace_id     TEXT
);

CREATE TABLE agent_approvals (
    id            TEXT PRIMARY KEY,
    principal_id  TEXT NOT NULL,
    op            TEXT NOT NULL,
    action_digest TEXT NOT NULL,
    scope         TEXT NOT NULL,
    params_json   TEXT NOT NULL,
    binding_json  TEXT NOT NULL,
    preview_json  TEXT NOT NULL,
    created_at    TEXT NOT NULL,
    expires_at    TEXT NOT NULL,
    decision      TEXT NOT NULL DEFAULT 'pending',
    decided_by    TEXT,
    decided_at    TEXT,
    consumed_at   TEXT,
    result_ref    TEXT
);
CREATE INDEX idx_approvals_digest ON agent_approvals(action_digest, decision);

-- Record of what personal data each principal was allowed to read.
CREATE TABLE data_disclosures (
    id           TEXT PRIMARY KEY,
    principal_id TEXT NOT NULL,
    subject_kind TEXT NOT NULL,
    subject_id   TEXT NOT NULL,
    fields       TEXT NOT NULL,
    at           TEXT NOT NULL
);

CREATE TABLE command_runs (
    id                 TEXT PRIMARY KEY,
    actor_id           TEXT NOT NULL,
    command            TEXT NOT NULL,
    args_redacted_json TEXT NOT NULL,
    request_id         TEXT NOT NULL,
    result             TEXT NOT NULL,
    dry_run            INTEGER NOT NULL DEFAULT 0,
    duration_ms        INTEGER NOT NULL,
    created_at         TEXT NOT NULL
);
CREATE INDEX idx_runs_actor ON command_runs(actor_id, created_at);

CREATE TABLE audit_log (
    seq              INTEGER PRIMARY KEY AUTOINCREMENT,
    id               TEXT NOT NULL UNIQUE,
    actor            TEXT NOT NULL,
    action           TEXT NOT NULL,
    timestamp        TEXT NOT NULL,
    redacted_subject TEXT NOT NULL,
    details_json     TEXT NOT NULL DEFAULT '{}',
    prev_hash        TEXT NOT NULL,
    integrity_hash   TEXT NOT NULL
);

CREATE TABLE idempotency_keys (
    actor_id      TEXT NOT NULL,
    command_scope TEXT NOT NULL,
    key           TEXT NOT NULL,
    request_hash  TEXT NOT NULL,
    response_json TEXT NOT NULL,
    expires_at    TEXT NOT NULL,
    PRIMARY KEY (actor_id, command_scope, key)
);

-- ------------------------------------------------------------------ jobs
CREATE TABLE jobs (
    id             TEXT PRIMARY KEY,
    kind           TEXT NOT NULL,
    state          TEXT NOT NULL,
    progress       INTEGER NOT NULL DEFAULT 0,
    params_json    TEXT NOT NULL DEFAULT '{}',
    result_json    TEXT,
    owner_actor_id TEXT NOT NULL,
    parent_job_id  TEXT,
    next_run_at    TEXT NOT NULL,
    attempts       INTEGER NOT NULL DEFAULT 0,
    max_attempts   INTEGER NOT NULL DEFAULT 3,
    retry_policy   TEXT NOT NULL DEFAULT 'exponential',
    cursor         TEXT,
    lease_owner    TEXT,
    lease_expires_at TEXT,
    cancel_requested INTEGER NOT NULL DEFAULT 0,
    error_code     TEXT,
    error_message  TEXT,
    created_at     TEXT NOT NULL,
    updated_at     TEXT NOT NULL,
    finished_at    TEXT
);
CREATE INDEX idx_jobs_state ON jobs(state, next_run_at);

CREATE TABLE job_events (
    id                   TEXT PRIMARY KEY,
    job_id               TEXT NOT NULL REFERENCES jobs(id) ON DELETE CASCADE,
    sequence             INTEGER NOT NULL,
    event_type           TEXT NOT NULL,
    payload_redacted_json TEXT NOT NULL DEFAULT '{}',
    at                   TEXT NOT NULL,
    UNIQUE (job_id, sequence)
);

CREATE TABLE schedules (
    id           TEXT PRIMARY KEY,
    kind         TEXT NOT NULL,
    params_json  TEXT NOT NULL DEFAULT '{}',
    interval_secs INTEGER NOT NULL,
    next_run_at  TEXT NOT NULL,
    enabled      INTEGER NOT NULL DEFAULT 1
);

CREATE TABLE workflows (
    id            TEXT PRIMARY KEY,
    name          TEXT NOT NULL UNIQUE,
    definition_json TEXT NOT NULL,
    digest        TEXT NOT NULL,
    created_by    TEXT NOT NULL,
    created_at    TEXT NOT NULL
);

CREATE TABLE workflow_runs (
    id            TEXT PRIMARY KEY,
    workflow_id   TEXT NOT NULL REFERENCES workflows(id),
    job_id        TEXT NOT NULL REFERENCES jobs(id),
    state         TEXT NOT NULL,
    checkpoint_json TEXT NOT NULL DEFAULT '{}',
    actor_id      TEXT NOT NULL,
    created_at    TEXT NOT NULL,
    updated_at    TEXT NOT NULL
);

-- Outbox of events for SSE/CLI watchers (transactional with the change).
CREATE TABLE events (
    seq        INTEGER PRIMARY KEY AUTOINCREMENT,
    topic      TEXT NOT NULL,
    payload    TEXT NOT NULL,
    at         TEXT NOT NULL
);
