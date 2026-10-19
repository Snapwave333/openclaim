# OpenClaim Assistant (browser extension)

Manifest V3 extension for Chrome/Edge. It does two things, both only on sites you approve one at a time:

1. **Prefill preview** — fetches a claim package the owner prepared in OpenClaim (`openclaim claims prepare --prefill name,email`), shows every candidate field with the value it would write and the value it would replace, fills only the ones you tick, and offers Undo. It never fills passwords, card/bank numbers, SSNs, signatures or CAPTCHA fields, and it never clicks Submit.
2. **Capture official page** — sends the page you are viewing (for example an ftc.gov refund page that blocks automated access) to OpenClaim as a user-provided snapshot with provenance.

## Pair

```powershell
openclaim agents register --name browser --kind extension --scope extension.prefill,sources.read --origin chrome-extension://<extension id>
```

Load the folder as an unpacked extension (`chrome://extensions` → Developer mode → Load unpacked), open the popup, paste the token.

The daemon only releases prefill values for the domain stored in the package (the official claim URL's host), and only to a paired extension principal whose origin was registered.

## Tests

```
npm ci
npm test
```

`test/fill-logic.test.js` proves with jsdom that blocked fields are never fillable, that only approved fields change, that undo restores previous values, and that the fill code contains no `submit`/`click` calls.
