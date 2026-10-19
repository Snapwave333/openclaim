# OpenClaim

Free, open-source, local-first settlement discovery and claims assistant. It finds consumer refund and settlement opportunities from free official sources, cites every claim it makes, assesses *possible* eligibility with a deterministic rules engine, organizes your evidence, walks you to the official filing site, and tracks deadlines and results.

**No AI inside.** OpenClaim ships zero language models, inference runtimes or AI API clients. External agents can drive it through a documented CLI and a model-free MCP server, under scopes and human approvals.

**Free means free.** No accounts, subscriptions, paid APIs, commissions or caps. Every dependency and every source is $0 and open.

**You file. OpenClaim never submits a claim**, completes a CAPTCHA, signs anything or touches payment details.

## Install (Windows 11, from source)

```powershell
cargo build --release --bin openclaim --bin openclaimd
# binaries: target\release\openclaim.exe, target\release\openclaimd.exe
```

Optional GUI: `cd apps/web && npm ci && npm run build`, then start the daemon with `--gui-dir apps/web/dist/web/browser` (or put the build in a `gui` folder next to `openclaimd.exe`).

## Five-minute start

```powershell
openclaim daemon ensure                       # starts openclaimd in the background
openclaim init                                # choose the owner passphrase
openclaim unlock                              # prints a $env:OPENCLAIM_SESSION line; run it
openclaim config set network.contact '"Your Name you@example.com"'   # required by SEC.gov fair-access rules
openclaim sources refresh --scope official    # queues compliant fetches (FTC, SEC, NAUPA)
openclaim jobs list
openclaim discover search --only-verified --sort deadline
```

`ftc.gov` answers automated clients with an anti-bot page. OpenClaim does not evade it: the source shows as **blocked**, and you can save the page from your own browser and import it with provenance:

```powershell
openclaim sources import-snapshot --source ftc-refunds --url https://www.ftc.gov/enforcement/refunds --file .\refunds.html
```

The browser extension (`apps/browser-extension`) can capture the page you are viewing with one click.

Then:

```powershell
openclaim people create --name "Ann Example" --consent '{"authorized_by_adult":true}'
openclaim people add-residence <person-id> --jurisdiction TX --start 2010-01-01
openclaim facts add --person <person-id> --type service --merchant Walmart --start 2020-05-05 --end 2020-05-05
openclaim match evaluate --person <person-id>
openclaim claims init --person <person-id> --opportunity <opportunity-id>
openclaim claims checklist <claim-id>
openclaim claims prepare --claim <claim-id> --prefill name,email
openclaim claims handoff --claim <claim-id>          # opens the official site; you complete and submit
openclaim claims record-submission --claim <claim-id> --confirmation ABC123 --user-confirmed
```

Everything is also available as `--output json` (exactly one envelope on stdout) or `--output ndjson`.

## For agents

```powershell
openclaim agents register --name researcher --scope discover.read,opportunities.read,sources.read,jobs.read
# give the printed token to the agent as OPENCLAIM_AGENT_TOKEN
openclaim mcp serve --transport stdio          # model-free MCP server for any harness
openclaim schema export --file command-schema.json
```

Sensitive actions return `APPROVAL_REQUIRED` (exit code 5) with an approval id. The owner reviews it (`openclaim approvals show <id>`, or the GUI) and the agent retries with `--approval <id>`. Approvals are digest-bound and single use. See [docs/agent-guide.md](docs/agent-guide.md).

## Documentation

- [Architecture](docs/architecture.md) — daemon, IPC, API, jobs, data model
- [CLI reference](docs/cli.md) and the machine contract `openclaim help-tree` / [docs/command-schema.json](docs/command-schema.json)
- [Source policy](docs/source-policy.md) — the free-only provider gate and connector lifecycle
- [Privacy and security](docs/privacy-security.md)
- [Agent guide](docs/agent-guide.md) — scopes, approvals, trust labels, MCP tools
- [Browser extension](apps/browser-extension/README.md)
- [Contributing](CONTRIBUTING.md)

## What this is not

Not a claims administrator, not a financial intermediary, not legal advice. Money goes directly from the official administrator to you. A "potential match" is computed from your own facts against quoted official terms; only the administrator decides eligibility.

## License

MIT. Test fixtures are snapshots of public U.S. government and NAUPA pages captured for parser tests.
