# OpenClaim

Free, open-source, local-first **class-action settlement** finder and claims assistant. It pulls open settlements from free public listings, verifies each one against its official settlement site, cites every claim it makes, assesses *possible* eligibility with a deterministic rules engine, organizes your evidence, walks you to the official filing site, and tracks deadlines and results.

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
openclaim sources refresh --scope all         # classaction.org listing + Stanford securities cases
openclaim jobs list
openclaim discover search --sort deadline     # listings (verification_pending until checked)
openclaim opportunities verify-official <id>  # one fetch of the settlement's own site → verified deadline
openclaim discover search --only-verified --sort deadline
```

If a site blocks automated reads, save the page in your browser and import it: `openclaim opportunities confirm-official <id> --url <page-url> --file .\saved.html`. The browser extension (`apps/browser-extension`) can capture the page you are viewing with one click.

Then:

```powershell
openclaim people create --name "Ann Example" --consent '{"authorized_by_adult":true}'
openclaim people add-residence <person-id> --jurisdiction TX --start 2010-01-01
openclaim facts add --person <person-id> --type purchase --merchant "Dr. Squatch" --start 2020-05-05 --end 2020-05-05
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
- [Source policy](docs/source-policy.md) — the free-only provider gate, bundled sources, official-site verification
- [Privacy and security](docs/privacy-security.md)
- [Agent guide](docs/agent-guide.md) — scopes, approvals, trust labels, MCP tools
- [Browser extension](apps/browser-extension/README.md)
- [Contributing](CONTRIBUTING.md)

## What this is not

Not a claims administrator, not a financial intermediary, not legal advice, and not a search of every settlement that exists. Money goes directly from the official administrator to you. A "potential match" is computed from your own facts against quoted official terms; only the administrator decides eligibility.

## License

MIT. Test fixtures are trimmed snapshots of public listing pages captured for parser tests.
