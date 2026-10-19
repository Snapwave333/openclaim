const $ = (id) => document.getElementById(id);
const bg = (msg) => chrome.runtime.sendMessage(msg);
let tab = null;
let plan = [];
let packageData = null;

function status(text, isError) {
  $("status").textContent = text;
  $("status").style.color = isError ? "#f472b6" : "";
}

async function activeTab() {
  const [t] = await chrome.tabs.query({ active: true, currentWindow: true });
  return t;
}

function originOf(url) {
  try {
    return new URL(url).origin;
  } catch {
    return null;
  }
}

async function hasSitePermission(origin) {
  return chrome.permissions.contains({ origins: [`${origin}/*`] });
}

async function ensureContent() {
  const origin = originOf(tab.url);
  if (!origin || !origin.startsWith("https://")) throw new Error("only https sites are supported");
  if (!(await hasSitePermission(origin))) throw new Error("click 'Allow on this site' first");
  await chrome.scripting.executeScript({ target: { tabId: tab.id }, files: ["src/fill-logic.js", "src/content.js"] });
}

function renderPlan() {
  const ul = $("plan");
  ul.textContent = "";
  if (!plan.length) {
    const li = document.createElement("li");
    li.textContent = "No matching non-sensitive fields found on this page.";
    ul.appendChild(li);
    $("fill").disabled = true;
    return;
  }
  for (const entry of plan) {
    const li = document.createElement("li");
    const cb = document.createElement("input");
    cb.type = "checkbox";
    cb.checked = true;
    cb.dataset.field = entry.field;
    cb.setAttribute("aria-label", `Fill ${entry.field}`);
    const text = document.createElement("div");
    const k = document.createElement("div");
    k.className = "k";
    k.textContent = `${entry.field} → "${entry.label}"`;
    const v = document.createElement("div");
    v.className = "v";
    v.textContent = entry.currentValue ? `${entry.value}  (replaces: ${entry.currentValue})` : entry.value;
    text.append(k, v);
    li.append(cb, text);
    ul.appendChild(li);
  }
  $("fill").disabled = false;
}

async function loadPackages() {
  const origin = originOf(tab.url);
  const sel = $("packages");
  sel.textContent = "";
  if (!origin) return;
  const r = await bg({ type: "bg:packages", origin });
  if (!r.ok) return status(r.error, true);
  for (const p of r.data.items) {
    const o = document.createElement("option");
    o.value = p.package_id;
    o.textContent = `${p.opportunity_title} (${(p.prefill_fields || []).join(", ") || "no fields"})`;
    sel.appendChild(o);
  }
  if (!r.data.items.length) {
    const o = document.createElement("option");
    o.textContent = "No package prepared for this site (run claims prepare)";
    o.value = "";
    sel.appendChild(o);
  }
}

async function loadSources() {
  const sel = $("sources");
  sel.textContent = "";
  const r = await bg({ type: "bg:sources" });
  if (!r.ok) return;
  const host = originOf(tab.url) ? new URL(tab.url).hostname : "";
  for (const s of r.data.items) {
    const hosts = s.allowed_hosts || [];
    if (!hosts.some((h) => host === h || host.endsWith(`.${h}`))) continue;
    const o = document.createElement("option");
    o.value = s.id;
    o.textContent = s.name;
    sel.appendChild(o);
  }
  $("capture").disabled = sel.options.length === 0;
  if (!sel.options.length) {
    const o = document.createElement("option");
    o.textContent = "This site is not a registered official source";
    sel.appendChild(o);
  }
}

async function init() {
  tab = await activeTab();
  $("site").textContent = tab && tab.url ? originOf(tab.url) || tab.url : "no active tab";
  $("extid").textContent = chrome.runtime.id;
  const s = await bg({ type: "bg:status" });
  $("pair-section").hidden = s.ok && s.paired;
  $("main-section").hidden = !(s.ok && s.paired);
  if (s.ok && s.paired) {
    await loadPackages();
    await loadSources();
  }
}

$("pair").addEventListener("click", async () => {
  const r = await bg({ type: "bg:pair", token: $("token").value, base: $("base").value });
  if (!r.ok) return status(r.error, true);
  $("token").value = "";
  status(`Paired as ${r.actor && r.actor.name ? r.actor.name : "extension"}.`);
  await init();
});

$("unpair").addEventListener("click", async () => {
  await bg({ type: "bg:unpair" });
  await init();
});

$("grant").addEventListener("click", async () => {
  const origin = originOf(tab.url);
  if (!origin) return status("no site", true);
  const ok = await chrome.permissions.request({ origins: [`${origin}/*`] });
  status(ok ? `Allowed on ${origin}` : "Permission not granted", !ok);
});

$("preview").addEventListener("click", async () => {
  try {
    const packageId = $("packages").value;
    if (!packageId) return status("prepare a claim package first", true);
    await ensureContent();
    const r = await bg({ type: "bg:package", packageId, origin: originOf(tab.url) });
    if (!r.ok) return status(r.error, true);
    packageData = r.data;
    const res = await chrome.tabs.sendMessage(tab.id, { type: "openclaim:plan", prefill: packageData.prefill });
    plan = res.plan || [];
    renderPlan();
    status(`${plan.length} field(s) can be filled. Untick any you do not want.`);
  } catch (e) {
    status(String(e.message || e), true);
  }
});

$("fill").addEventListener("click", async () => {
  const approved = Array.from(document.querySelectorAll("#plan input[type=checkbox]:checked")).map((c) => c.dataset.field);
  const res = await chrome.tabs.sendMessage(tab.id, { type: "openclaim:apply", plan, approved });
  $("undo").disabled = !(res && res.filled);
  status(`Filled ${res ? res.filled : 0} field(s). Check them, then submit the form yourself.`);
});

$("undo").addEventListener("click", async () => {
  const res = await chrome.tabs.sendMessage(tab.id, { type: "openclaim:undo" });
  $("undo").disabled = true;
  status(`Restored ${res ? res.restored : 0} field(s).`);
});

$("capture").addEventListener("click", async () => {
  try {
    const sourceId = $("sources").value;
    if (!sourceId) return;
    await ensureContent();
    const page = await chrome.tabs.sendMessage(tab.id, { type: "openclaim:capture" });
    const r = await bg({ type: "bg:capture", sourceId, url: page.url, html: page.html });
    if (!r.ok) return status(r.error, true);
    status(`Captured: ${r.data.parsed} record(s) parsed, ${r.data.summary.inserted} new, ${r.data.summary.updated} updated.`);
  } catch (e) {
    status(String(e.message || e), true);
  }
});

init().catch((e) => status(String(e.message || e), true));
