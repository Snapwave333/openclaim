/** Envelope and domain types for the OpenClaim localhost API (MDR §11, §16). */

export type ErrorCode =
  | 'INVALID_ARGUMENT'
  | 'NOT_FOUND'
  | 'PERMISSION_DENIED'
  | 'APPROVAL_REQUIRED'
  | 'CONFLICT'
  | 'UNAVAILABLE'
  | 'TIMEOUT'
  | 'POLICY_BLOCKED'
  | 'PARTIAL_RESULT'
  | 'INTERNAL';

export interface ApiErrorBody {
  code: ErrorCode | string;
  message: string;
  retryable: boolean;
  approval_id?: string | null;
  needed_scopes?: string[] | null;
  next_action?: string | null;
  details?: unknown;
}

export interface Pagination {
  limit: number;
  next_cursor?: string | null;
  total?: number | null;
  snapshot?: string | null;
}

export interface Envelope<T = unknown> {
  schema_version: string;
  ok: boolean;
  request_id: string;
  trace_id: string;
  data?: T;
  error?: ApiErrorBody | null;
  warnings?: string[];
  pagination?: Pagination | null;
  audit_event_id?: string | null;
  dry_run?: boolean;
}

/** Thrown by ApiService when `ok` is false (or the transport fails). */
export class ApiError extends Error {
  constructor(
    public readonly code: string,
    message: string,
    public readonly status: number,
    public readonly body?: ApiErrorBody,
    public readonly requestId?: string,
  ) {
    super(message);
    this.name = 'ApiError';
  }
  get retryable(): boolean { return this.body?.retryable ?? false; }
  get approvalId(): string | null | undefined { return this.body?.approval_id; }
}

export type VerificationState = 'verified' | 'verification_pending' | 'stale' | 'disputed' | string;
export type MatchState = 'potential_match' | 'needs_more_information' | 'unlikely' | 'not_applicable' | 'manual_review';

export interface Trust {
  primary_source: boolean;
  verified_on: string | null;
  verification_state: VerificationState;
  claim_deadline: string | null;
  deadline_verified: boolean;
  eligibility_note: string;
}

export interface Benefit {
  amount_known: boolean;
  description?: string | null;
  amount_minor?: number | null;
  currency?: string | null;
}

export interface Opportunity {
  id: string;
  title: string;
  opp_type: string;
  administrator: string | null;
  administrator_phone?: string | null;
  case_number: string | null;
  court?: string | null;
  official_url: string;
  claim_url: string | null;
  status: string;
  verification_state: VerificationState;
  authority_tier: string;
  claim_deadline: string | null;
  deadline_verified: boolean;
  class_period_start: string | null;
  class_period_end: string | null;
  jurisdictions: string[];
  summary: string | null;
  benefit: Benefit | null;
  proof_required: string | null;
  claim_method: string | null;
  source_updated_on: string | null;
  source_id: string;
  source_name?: string;
  verified_at: string | null;
  updated_at: string;
  first_seen_at: string;
  version: number;
  bookmarked: boolean;
  hidden: boolean;
  source_health: string;
  trust: Trust;
}

export interface UntrustedText {
  trust: 'untrusted_source_text' | string;
  origin: string;
  notice: string;
  text: string;
}

export interface EvidenceItem {
  id: string;
  assertion_type: string;
  precise_locator: string;
  excerpt: UntrustedText;
  created_at: string;
  snapshot_url: string;
  fetched_at: string;
  acquisition: string;
  content_hash: string;
  parser_version: string;
}

export interface OpportunityDetail {
  opportunity: Opportunity;
  rules?: { version: number | null; rules: unknown; note?: string };
  provenance?: { evidence: EvidenceItem[] };
  sources?: unknown;
  changes?: { items?: ChangeItem[] } | ChangeItem[];
}

export interface ChangeItem {
  opportunity_id: string;
  title: string;
  field: string;
  old_value: string | null;
  new_value: string | null;
  detected_at: string;
}

export interface Person {
  id: string;
  display_name: string;
  consent?: { authorized_by_adult: boolean; recorded_at: string; note: string | null };
  residences?: Residence[];
  details?: PersonDetails;
  consumer_fact_count?: number;
  updated_at: string;
  version: number;
}

export interface Residence { id?: string; jurisdiction: string; start?: string | null; end?: string | null }

export interface PersonDetails {
  aliases: string[];
  emails: string[];
  phones: string[];
  mailing_address: string | null;
  employers: string[];
  purchase_categories: string[];
  memberships: string[];
  notes: string | null;
}

export interface MatchItem {
  opportunity_id: string;
  title: string;
  status: string;
  claim_deadline: string | null;
  deadline_verified: boolean;
  verification_state: VerificationState;
  official_url: string;
  assessment: MatchState;
  stale: boolean;
  rules_version: number | null;
  evaluated_at: string;
}

export interface Assessment {
  state: MatchState;
  summary: string;
  reasons: Array<{ text?: string; rule?: string; result?: string; [k: string]: unknown } | string>;
  missing_facts: Array<{ key?: string; question?: string; label?: string; [k: string]: unknown } | string>;
  disclaimer: string;
}

export interface MatchExplain {
  person_id: string;
  opportunity: { id: string; title: string; official_url: string; claim_deadline: string | null; deadline_verified: boolean; verification_state: VerificationState };
  rules_version: number | null;
  assessment: Assessment;
  trust: string;
}

export interface Claim {
  id: string;
  person_id: string;
  opportunity_id: string;
  status: string;
  submitted_at: string | null;
  external_confirmation_id: string | null;
  manual_paid_amount: number | null;
  currency: string | null;
  display_name: string;
  created_at: string;
  updated_at: string;
  version: number;
  opportunity_title: string;
  official_url: string;
  claim_url: string | null;
  claim_deadline: string | null;
  deadline_verified: boolean;
  opportunity_status: string;
  verification_state: VerificationState;
  claim_method: string | null;
  proof_required: string | null;
  notes: string | null;
  allowed_next: string[];
}

export interface ClaimEvent { event_type: string; from_status: string | null; to_status: string; who_recorded: string; timestamp: string; evidence_id: string | null; metadata: string }

export interface ClaimDetail { claim: Claim; events: ClaimEvent[]; documents: unknown[]; packages: unknown[] }

export interface Checklist {
  claim_id: string;
  status: string;
  official_url: string;
  claim_url: string | null;
  deadline: string | null;
  deadline_verified: boolean;
  eligibility: Assessment;
  linked_documents: unknown[];
  missing_facts: unknown[];
  warnings: string[];
  ready: boolean;
  next_steps: string[];
}

export interface Handoff {
  claim_id: string;
  open_url: string;
  package_id: string | null;
  allowed_domain: string;
  instructions: string[];
  note: string;
}

export interface SourceHealth {
  id: string;
  name: string;
  health: string;
  enabled: boolean;
  last_ok_at: string | null;
  next_fetch_at: string | null;
  last_failure: string | null;
  last_failure_detail: string | null;
  consecutive_failures: number;
}

export interface Source extends SourceHealth {
  url: string;
  connector: string;
  authority_tier: string;
  trust_status: string;
  disabled_reason: string | null;
  last_attempt_at: string | null;
  created_at: string;
  version: number;
}

export interface Alert {
  id: string;
  subject_kind: string;
  subject_id: string | null;
  kind: string;
  title: string;
  due_at: string;
  disposition: string;
  snoozed_until: string | null;
  opportunity_title?: string | null;
  claim_deadline?: string | null;
}

export interface Job {
  id: string;
  kind: string;
  state: string;
  progress: number;
  owner_actor_id: string;
  attempts: number;
  max_attempts: number;
  error_code: string | null;
  error_message: string | null;
  created_at: string;
  updated_at: string;
  finished_at: string | null;
}

export interface Agent {
  id: string;
  display_name: string;
  kind: string;
  scopes: string[];
  allowed_origin: string | null;
  created_at: string;
  expires_at: string | null;
  revoked_at: string | null;
}

export interface Scope { scope: string; description: string; owner_only: boolean }

export interface Approval {
  id: string;
  agent_id?: string;
  agent_name?: string;
  op: string;
  summary?: string;
  params?: unknown;
  params_redacted?: unknown;
  action_digest: string;
  status: string;
  requested_at?: string;
  expires_at?: string | null;
  decided_at?: string | null;
  decision?: string | null;
  note?: string | null;
  [k: string]: unknown;
}

export interface Doc {
  id: string;
  name: string;
  doc_type: string;
  mime: string;
  size_bytes: number;
  classification: string | null;
  extraction_state: string;
  extraction_note: string | null;
  text_chars: number;
  suggestion_count: number;
  person_id: string | null;
  links: Array<{ target_kind: string; target_id: string; locator: string | null; validated: boolean }>;
  imported_at: string;
  version: number;
}

export interface Suggestion { kind: string; value: string; span_start: number; span_end: number; excerpt: string }

export interface ConfigEntry { value: unknown; default: unknown; description: string }

export interface OwnerStatus { initialized: boolean; unlocked: boolean; actor: { kind: string; session_id?: string } }

export interface SseEvent<T = unknown> { seq: number; topic: string; payload: T; at: string }

export interface Paged<T> { items: T[]; [k: string]: unknown }
