# Privacy and security

## Data stays local

No accounts, telemetry, analytics, crash reporting or cloud sync. The only outbound traffic is to allowlisted free official sources, and only when you (or the schedule you can turn off with `network.offline=true`) ask for it. Personal documents are never transmitted anywhere; you submit materials to official sites yourself.

## Encryption

- A 256-bit vault key is generated on first run and stored DPAPI-protected (Windows) or in an owner-only file (Unix).
- Vault files use XChaCha20-Poly1305; sensitive profile details, claim notes, document metadata and prefill packages are encrypted columns.
- Recovery limitation: losing the Windows account or the key file makes encrypted evidence unrecoverable. `openclaim data backup` copies the database and vault files; keep `vault.key.dpapi` with backups meant for another machine.

## Owner passphrase and sessions

`openclaim init` sets an argon2-hashed passphrase. It gates approvals, personal data, agent management and the GUI. `openclaim unlock` creates a memory-only session (default 60 min, five attempts per minute). Sessions end on daemon restart, passphrase change and wipe.

## Local boundary

- IPC: Windows named pipe with an owner-only DACL (`D:P(A;;GA;;;OW)(A;;GA;;;SY)`), remote clients rejected; Unix socket mode 0600.
- HTTP: bound to 127.0.0.1; Host allowlist (DNS rebinding), Origin allowlist (local GUI and paired extension origins only), HttpOnly SameSite=Strict cookie plus `X-OpenClaim-CSRF` header for mutations, strict CSP, 12 MB body limit.
- Same-account agents: executing the binary grants nothing personal. Agents need a registered token; sensitive actions need a human approval bound to the exact parameters and record versions.

## Untrusted content

Website text and document text are data. They are stripped of control/invisible/bidi characters, bounded, and returned under the label `untrusted_source_text`. Nothing in them can add tools, grant scopes or change instructions. Document previews are plain text; HTML, scripts, executables and archives are refused at import; PDFs are parsed in a panic-guarded path.

## Network fetching

https only; no IP literals or embedded credentials; DNS results in private/loopback/link-local ranges are rejected; redirects only within the connector allowlist; bodies capped at 6 MB; robots.txt honoured; per-host spacing and `Retry-After` respected.

## Redaction

SSNs, 9-digit ids, card/account numbers, emails and phone numbers are redacted from logs, audit details, job payloads, agent-facing suggestions and the document text index. Tokens are recognizable by prefix (`ocs_`, `oca_`, `ocx_`) and redacted wherever they appear.

## Audit

A hash-chained `audit_log` records every mutation, approval, unlock and disclosure; `openclaim audit verify` recomputes the chain. `openclaim agents audit <id>` lists what an agent ran, read and proposed.

## Deleting data

`openclaim data wipe --confirm WIPE` erases personal tables and vault files (add `--include-catalog` to drop the downloaded catalog). `people delete` and `documents delete` are hard deletes.

## Reporting security issues

Open a private security advisory on the repository or email the maintainer listed in `Cargo.toml`. Please do not file public issues for vulnerabilities.
