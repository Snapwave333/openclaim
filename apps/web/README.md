# OpenClaim web GUI (`apps/web`)

Angular 22 (standalone components, signals, strict TypeScript) client for the `openclaimd` localhost API.
It is optional: the CLI and MCP adapter work without it. No AI, no analytics, no CDN or external fonts; it
works fully offline. Only the Angular CDK (free) is used for accessible dialogs.

## Layout

```
src/app/core      api.service.ts (envelope + CSRF client), auth.service.ts, events.service.ts (SSE),
                  prefs.service.ts (theme/font/motion in localStorage), toast.service.ts, format.ts, glossary.ts
src/app/shared    trust-display, untrusted-text, badge, term (glossary tooltip), opportunity-card,
                  confirm dialog (CDK), person selector
src/app/pages     login, overview, discover (+ detail), matches, claims (+ detail), property, vault,
                  sources, agents, approvals, settings
e2e/              Playwright specs (login/discover/detail/bookmark, screens, axe accessibility smoke)
```

Security notes: the CSRF token lives only in `ApiService` memory (never localStorage); a page reload
therefore asks for the passphrase again even if the `oc_session` cookie is still valid. Third-party text
(evidence excerpts, document text) is rendered through `<oc-untrusted-text>` as plain text in a distinct
style; nothing is ever bound to `innerHTML`. Agent proposals carry an "agent proposed" label until confirmed.
The GUI never submits anything to an official site; it only opens official URLs in a new tab (`rel=noopener`).

## Prerequisites

- Node 20+ and npm.
- A built daemon: from the repo root `cargo build` (binaries in `target/debug/`).

## Run a development daemon with seed data

```powershell
# PowerShell, from the repo root
$env:OPENCLAIM_DEV_ORIGINS = "http://localhost:4200"      # lets `ng serve` (port 4200) talk to the daemon
$env:OPENCLAIM_HOME = "$env:LOCALAPPDATA\Temp\oc-gui-dev"
target\debug\openclaimd.exe --data-dir $env:OPENCLAIM_HOME --port 7865 --verbose
```

In a second shell (same `OPENCLAIM_HOME`):

```powershell
echo correct-horse | target\debug\openclaim.exe init --passphrase-stdin
$s = (echo correct-horse | target\debug\openclaim.exe unlock --passphrase-stdin --output json | ConvertFrom-Json).data.session
$env:OPENCLAIM_SESSION = $s
target\debug\openclaim.exe sources import-snapshot --source ftc-refunds --url https://www.ftc.gov/enforcement/refunds --file test-fixtures\html\ftc_refunds_index.html
target\debug\openclaim.exe sources import-snapshot --source ftc-refunds --url https://www.ftc.gov/enforcement/refunds/walmart-refunds --file test-fixtures\html\ftc_refund_walmart.html
target\debug\openclaim.exe people create --name "Ann Example" --consent '{"authorized_by_adult":true}'
```

Stop it with `target\debug\openclaim.exe daemon stop`.

## Develop

```bash
cd apps/web
npm ci
npm start            # ng serve on http://localhost:4200, /api proxied to 127.0.0.1:7865 (proxy.conf.json)
```

If the daemon runs on another port, change `target` in `proxy.conf.json` and `--port`.

## Build and serve from the daemon

```bash
npm run build        # -> apps/web/dist/web/browser
```

```powershell
target\debug\openclaimd.exe --data-dir $env:OPENCLAIM_HOME --port 7865 --gui-dir apps\web\dist\web\browser
# open http://127.0.0.1:7865/
```

The daemon sends a strict CSP (`script-src 'self'`, inline styles allowed). The production build therefore
disables Angular's critical-CSS inlining (it would inject an inline `<script>`), see `angular.json`.

## Tests

```bash
npm test             # unit tests (Vitest + jsdom, Angular's default runner), runs once, headless
npm run e2e          # Playwright against the seeded daemon on 7865 (starts `ng serve` on 4200 itself)
BASE_URL=http://127.0.0.1:7865 npm run e2e   # same suite against the build served by the daemon
```

First time: `npx playwright install chromium`.

The e2e suite logs in with the seed passphrase `correct-horse` (override with `OPENCLAIM_E2E_PASSPHRASE`).
The daemon allows five unlock attempts per minute, so each spec file logs in once on a shared page; the
helper waits out the window if a previous run just used it. The suite leaves the seed as it found it
(the bookmark toggled in the test is toggled back).

Accessibility: `e2e/a11y.spec.ts` runs axe-core (WCAG 2.x A/AA) on the login, overview and discover pages
(discover in both dark and light themes) and fails on any violation.

## Preferences

Theme (system/dark/light), font size and motion are stored in `localStorage` under `openclaim.prefs`.
Nothing else is stored client-side.
