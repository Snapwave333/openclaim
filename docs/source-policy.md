# Source policy

OpenClaim fetches only free, public, official-or-clearly-identified sources, and only where automated access is permitted.

## Provider gate

Every connector is declared by a manifest in `source-manifests/` (or registered by the owner). A manifest must state:

| Field | Rule |
|---|---|
| `price` | must be `"free"`; anything else is refused (`POLICY_BLOCKED`) |
| `auth_requirement` | `none` or `declared_contact_user_agent` (SEC.gov fair access) |
| `robots_policy` | must be `"respect"` |
| `terms_url`, `terms_review_date` | a human reviewed the terms on that date |
| `rate_limit` | `min_interval_ms` ≥ 500 and a per-run request cap |
| `automation_allowed` | `false` means only user-saved snapshots are accepted |
| `allowed_hosts` | the only hosts the fetcher may contact or follow redirects to |
| `freshness_ttl_hours` | how long a successful fetch stays fresh |

If a source starts answering with anti-automation pages, rate limits or a paywall, the connector is marked `blocked`/`degraded`, previously verified data is kept with a freshness warning (records become `stale` after 3× TTL), and the official link is shown for manual lookup.

## Bundled sources (Tier A)

- **FTC Refund Programs** — index table + program pages. ftc.gov currently returns HTTP 403 to non-browser clients; the connector reports `blocked` and the CLI/extension offer snapshot import.
- **SEC Distributions to Harmed Investors** — case list + case pages. Requires `network.contact` so the User-Agent declares a contact, per SEC's fair-access policy; 1 request/second.
- **NAUPA directory** — official unclaimed-property programs for 50 states + DC, territories and Canada. A snapshot captured 2026-10-19 is bundled so the directory works offline from first run; it is refreshed weekly.

Tier B (feeds, tips, user-imported notices) only produce **quarantined tips** until a primary source corroborates them.

## Connector lifecycle

`discover → fetch (conditional, bounded, allowlisted) → parse (fixtures-tested) → normalize → dedupe → verify → publish → refresh → expire/archive`

Dedupe key = normalized title + docket + administrator + class-period start. Records merge only when the title matches and no present identifier conflicts; the same company with different dockets or administrators never merges.

Verification: a record is `verified` only when a Tier A detail page confirmed the terms. Index rows alone are `verification_pending`. Deadlines are `deadline_verified` only when parsed from the primary page. Change detection records field diffs, marks matches stale and raises alerts for tracked opportunities.

## What is never done

No paid court-record retrieval (PACER), no proprietary datasets, no CAPTCHA solving, no proxies, no header spoofing, no bulk name searches on state portals, no claims of coverage that the sources do not support.
