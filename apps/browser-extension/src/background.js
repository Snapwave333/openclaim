// Service worker: holds the pairing token and talks to the local daemon.
// The daemon only releases prefill values for the domain the owner approved.

const DEFAULT_BASE = "http://127.0.0.1:7865";

async function settings() {
  const s = await chrome.storage.local.get(["token", "base"]);
  return { token: s.token || "", base: s.base || DEFAULT_BASE };
}

async function callOp(op, params) {
  const { token, base } = await settings();
  if (!token) throw new Error("not paired: paste the extension token from `openclaim agents register --kind extension`");
  const resp = await fetch(`${base}/api/v1/ops/${op}`, {
    method: "POST",
    headers: { "content-type": "application/json", authorization: `Bearer ${token}` },
    body: JSON.stringify({ params }),
  });
  const env = await resp.json();
  if (!env.ok) {
    const e = env.error || {};
    throw new Error(`${e.code || "ERROR"}: ${e.message || "request failed"}`);
  }
  return env.data;
}

chrome.runtime.onMessage.addListener((msg, _sender, reply) => {
  (async () => {
    try {
      switch (msg.type) {
        case "bg:pair": {
          await chrome.storage.local.set({ token: msg.token.trim(), base: (msg.base || DEFAULT_BASE).replace(/\/$/, "") });
          const me = await callOp("owner.status", {});
          reply({ ok: true, actor: me.actor });
          break;
        }
        case "bg:unpair":
          await chrome.storage.local.remove(["token"]);
          reply({ ok: true });
          break;
        case "bg:status": {
          const s = await settings();
          reply({ ok: true, paired: !!s.token, base: s.base });
          break;
        }
        case "bg:packages":
          reply({ ok: true, data: await callOp("claims.packages_for_origin", { origin: msg.origin }) });
          break;
        case "bg:package":
          reply({ ok: true, data: await callOp("claims.prefill_package", { package_id: msg.packageId, origin: msg.origin }) });
          break;
        case "bg:sources":
          reply({ ok: true, data: await callOp("sources.list", {}) });
          break;
        case "bg:capture":
          reply({ ok: true, data: await callOp("sources.capture_from_extension", { source_id: msg.sourceId, url: msg.url, html: msg.html }) });
          break;
        default:
          reply({ ok: false, error: "unknown message" });
      }
    } catch (e) {
      reply({ ok: false, error: String(e.message || e) });
    }
  })();
  return true;
});
