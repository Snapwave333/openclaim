// Injected on demand (user click in the popup) into a site the user approved.
// It only answers messages from the extension; it never submits forms.

(function () {
  if (window.__openclaimContentReady) return;
  window.__openclaimContentReady = true;
  const F = window.OpenClaimFill;
  let lastUndo = [];

  function overlay(text) {
    let box = document.getElementById("openclaim-overlay");
    if (!box) {
      box = document.createElement("div");
      box.id = "openclaim-overlay";
      box.setAttribute("role", "status");
      box.style.cssText = "position:fixed;right:12px;bottom:12px;z-index:2147483647;max-width:360px;padding:10px 12px;border-radius:8px;background:#0b0f14;color:#e6edf3;font:13px/1.4 system-ui,sans-serif;box-shadow:0 4px 18px rgba(0,0,0,.4);border:1px solid #22d3ee55";
      document.documentElement.appendChild(box);
    }
    box.textContent = text;
    clearTimeout(box.__t);
    box.__t = setTimeout(() => box.remove(), 8000);
  }

  chrome.runtime.onMessage.addListener((msg, _sender, reply) => {
    if (!msg || typeof msg !== "object") return;
    if (msg.type === "openclaim:plan") {
      reply({ ok: true, plan: F.planFill(document, msg.prefill || {}), origin: location.origin });
    } else if (msg.type === "openclaim:apply") {
      lastUndo = F.applyFill(document, msg.plan || [], msg.approved || []);
      overlay(`OpenClaim filled ${lastUndo.length} field(s). Review them, then submit the form yourself. Use the popup to undo.`);
      reply({ ok: true, filled: lastUndo.length });
    } else if (msg.type === "openclaim:undo") {
      const n = F.undoFill(document, lastUndo);
      lastUndo = [];
      overlay(`OpenClaim restored ${n} field(s).`);
      reply({ ok: true, restored: n });
    } else if (msg.type === "openclaim:capture") {
      reply({ ok: true, url: location.href, html: document.documentElement.outerHTML.slice(0, 6 * 1024 * 1024) });
    }
    return true;
  });
})();
