# CLI reference

`openclaim --help` shows the tree; `openclaim help-tree --output json` prints it with argument metadata; `openclaim schema export` prints every operation's parameter schema. This page summarizes the contracts.

## Global flags

`--output table|text|json|ndjson` · `--fields a,b` · `--limit N` · `--cursor C` · `--timeout S` · `--offline` · `--dry-run` · `--request-id ID` · `--idempotency-key K` · `--non-interactive` · `--approval ID` · `--profile NAME` · `--data-dir PATH` · `--session TOKEN` (env `OPENCLAIM_SESSION`) · `--agent-token TOKEN` (env `OPENCLAIM_AGENT_TOKEN`) · `--no-autostart` (env `OPENCLAIM_NO_AUTOSTART`)

## Output contract

- `--output json`: stdout is exactly one envelope object. stderr carries diagnostics only.
- `--output ndjson`: one JSON object per line; `jobs watch` and `discover watch` stream events.
- Human modes print tables/key-values on stdout; warnings, errors, next actions and `more results: --cursor …` on stderr.
- `--dry-run` never commits, never queues jobs, never writes files, never starts network fetches; the envelope has `dry_run: true` and names network work it skipped.
- `--non-interactive` (automatic when stdin is not a terminal) turns prompts into `APPROVAL_REQUIRED` / `INVALID_ARGUMENT`.

## Exit codes

| Code | Class |
|---|---|
| 0 | OK (or job accepted) |
| 2 | INVALID_ARGUMENT |
| 3 | NOT_FOUND |
| 4 | PERMISSION_DENIED |
| 5 | APPROVAL_REQUIRED |
| 6 | CONFLICT (stale version, idempotency mismatch) |
| 7 | UNAVAILABLE (daemon/source) |
| 8 | TIMEOUT (a durable job may continue) |
| 9 | POLICY_BLOCKED (paid, disallowed, unsafe) |
| 10 | PARTIAL_RESULT |
| 11 | INTERNAL |

## Namespaces

`daemon` ensure/start/stop/restart/status/health/logs/doctor · `discover` search/new/trending/compare/watch/history · `opportunities` list/show/rules/set-rules/sources/changes/verify-official/confirm-official/verify/bookmark/hide/report/export · `tips` submit/list · `sources` list/show/health/refresh/test/validate/register/disable/enable/history/import-snapshot · `match` evaluate/explain/list/missing/refresh/compare/export · `people` list/show/create/update/residences/add-residence/remove-residence/consent/history/delete · `facts` list/add/propose/validate/confirm/reject/link-proof/remove/attest/exclude/history · `documents` import/list/show/extract/suggestions/search/excerpt/link/redact/export/delete · `claims` list/show/init/checklist/prepare/handoff/record-submission/update/history/export/ledger/prefill-package · `alerts` list/due/add/snooze/dismiss/export-ics · `jobs` list/show/enqueue/watch/wait/cancel/retry/history/kinds · `workflows` list/validate/register/plan/run/resume/status/cancel/export · `agents` list/register/show/authorize/scopes/sessions/revoke/audit · `approvals` list/show/request/approve/deny/expire · `audit` list/show/export/verify · `data` backup/restore/export/import/integrity/wipe · `config` show/get/set/validate/reset · `schema` export/commands/describe/version/validate · `mcp` serve/inspect/list-tools/test · `completion` powershell/bash/zsh · `gui` open · `events` poll · `init` · `unlock` · `lock` · `whoami`

## Filter vocabulary

`--filter "status=open only_verified deadline_before=2027-01-01 state=CA sort=deadline"` or the typed flags: `--status`, `--state`, `--only-verified`, `--deadline-after/before`, `--verified-since`, `--claim-method`, `--proof-required`, `--administrator`, `--source`, `--source-health`, `--match-state` (with `--person`), `--bookmarked`, `--include-hidden`, `--sort deadline|updated|verified|title|activity`.

## Completions

```powershell
openclaim completion powershell | Out-String | Invoke-Expression
```
