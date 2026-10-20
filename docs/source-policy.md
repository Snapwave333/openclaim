# Source policy

OpenClaim finds **class-action settlements** from free public listings and only where automated access is permitted. It never pays for data, never evades blocks, and never claims a listing is verified until the settlement's own official site has been checked.

## Provider gate

Every connector is declared by a manifest in `source-manifests/` (or registered by the owner). A manifest must state:

| Field | Rule |
|---|---|
| `price` | must be `"free"`; anything else is refused (`POLICY_BLOCKED`) |
| `auth_requirement` | `none` or `declared_contact_user_agent` (sites with fair-access rules) |
| `robots_policy` | must be `"respect"` |
| `terms_url`, `terms_review_date` | a human reviewed the terms on that date |
| `rate_limit` | `min_interval_ms` ≥ 500 and a per-run request cap |
| `automation_allowed` | `false` means only user-saved snapshots are accepted |
| `allowed_hosts` | the only hosts the fetcher may contact or follow redirects to |
| `freshness_ttl_hours` | how long a successful fetch stays fresh |

If a source starts answering with anti-automation pages, rate limits or a paywall, the connector is marked `blocked`/`degraded`, previously fetched data is kept with a freshness warning (records become `stale` after 3× TTL), and the official link is shown for manual lookup.

## Bundled sources

| Source | Tier | What it gives | Access |
|---|---|---|---|
| **classaction.org open settlements** (`classaction-org`) | B (listing) | ~200 open settlements: name, official administrator site, deadline, payout note, proof requirement, one-sentence class description | robots allow; 1 request per run |
| **Stanford Securities Class Action Clearinghouse** (`stanford-scac`) | A | recent federal securities class actions with case pages (status, filing date, summary) | robots allow; 2 s spacing, ≤25 case pages per run |

Tried and not bundled: topclassactions.com and kroll.com answer automated clients with HTTP 403 (blocked, not evaded); jndla.com renders its case list client-side from a 5 MB CMS payload; Epiq and Angeion publish no stable public case index. A paired browser extension can still capture any of those pages as a user-provided snapshot.

## Verification: listing → official site

A listing is a lead. A record becomes **verified** only when its own official settlement page has been checked:

- `openclaim opportunities verify-official <id>` queues one fetch of the record's official URL. The fetcher allows only that host, https only, SSRF-safe DNS, no cross-host redirects, bounded body.
- `openclaim opportunities confirm-official <id> --url <page> --file saved.html` does the same from a page you saved in your browser (for sites that block automated reads).
- The page must name the settlement (at least two significant title terms). A claim deadline found on the page becomes the **verified deadline**; class periods are recorded; the snapshot and quoted sentences become evidence links.
- Deadline reminders are only ever scheduled from verified deadlines.

## Connector lifecycle

`discover → fetch (conditional, bounded, allowlisted) → parse (fixtures-tested) → normalize → dedupe → verify on official site → publish → refresh → expire/archive`

Dedupe key = normalized title + docket + administrator + class-period start. Records merge only when the title matches **and** at least one strong identifier is present on both sides and agrees; a bare company-name match never merges.

## What is never done

No paid court-record retrieval (PACER), no proprietary datasets, no CAPTCHA solving, no proxies, no header spoofing, no claims of coverage that the sources do not support.
