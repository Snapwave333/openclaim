// Pure form-field logic shared by the content script and the tests.
// No network, no DOM mutation beyond what `applyFill`/`undoFill` do explicitly.
// Invariants (MDR §5.5, §12): never touch password/payment/SSN/signature/CAPTCHA
// fields; never submit a form; every fill is previewed and undoable.

(function (root, factory) {
  if (typeof module === "object" && module.exports) {
    module.exports = factory();
  } else {
    root.OpenClaimFill = factory();
  }
})(typeof self !== "undefined" ? self : this, function () {
  "use strict";

  const BLOCKED = /pass(word|code|phrase)|ssn|social|security|tax\s*id|tin\b|ein\b|card|cvv|cvc|expir|routing|account\s*(no|num|#)|acct|iban|swift|bank|signature|sign\b|captcha|otp|one[- ]time|pin\b|secret|dob|birth|license|passport/i;

  const CLASSIFIERS = [
    { field: "email", autocomplete: ["email"], pattern: /e-?mail/i, types: ["email"] },
    { field: "phone", autocomplete: ["tel", "tel-national"], pattern: /phone|tel|mobile|cell/i, types: ["tel"] },
    { field: "name", autocomplete: ["name"], pattern: /^(full[\s_-]*)?name$|full[\s_-]*name|your[\s_-]*name|claimant[\s_-]*name|applicant/i, types: [] },
    { field: "mailing_address", autocomplete: ["street-address", "address-line1"], pattern: /mailing[\s_-]*address|street|address(?!.*e-?mail)|addr(ess)?[\s_-]*(line)?[\s_-]*1?$/i, types: [] },
  ];

  const FILLABLE_TYPES = new Set(["text", "email", "tel", "search", "url", ""]);

  function labelText(input, doc) {
    const parts = [];
    if (input.id) {
      const byFor = doc.querySelector(`label[for="${cssEscape(input.id)}"]`);
      if (byFor) parts.push(byFor.textContent || "");
    }
    const parent = input.closest ? input.closest("label") : null;
    if (parent) parts.push(parent.textContent || "");
    const aria = input.getAttribute("aria-label");
    if (aria) parts.push(aria);
    const labelled = input.getAttribute("aria-labelledby");
    if (labelled) {
      labelled.split(/\s+/).forEach((id) => {
        const el = doc.getElementById(id);
        if (el) parts.push(el.textContent || "");
      });
    }
    return parts.join(" ").replace(/\s+/g, " ").trim();
  }

  function cssEscape(s) {
    return String(s).replace(/["\\]/g, "\\$&");
  }

  function describe(input, doc) {
    return {
      name: input.getAttribute("name") || "",
      id: input.id || "",
      placeholder: input.getAttribute("placeholder") || "",
      autocomplete: (input.getAttribute("autocomplete") || "").toLowerCase().trim(),
      type: (input.getAttribute("type") || "").toLowerCase(),
      label: labelText(input, doc),
    };
  }

  function isFillable(meta, input) {
    const tag = input.tagName ? input.tagName.toLowerCase() : "";
    if (tag !== "input" && tag !== "textarea") return false;
    if (tag === "input" && !FILLABLE_TYPES.has(meta.type)) return false;
    if (input.disabled || input.readOnly) return false;
    const haystack = `${meta.name} ${meta.id} ${meta.placeholder} ${meta.autocomplete} ${meta.label}`;
    if (BLOCKED.test(haystack)) return false;
    return true;
  }

  function classify(meta) {
    for (const c of CLASSIFIERS) {
      if (meta.autocomplete && c.autocomplete.includes(meta.autocomplete)) return c.field;
    }
    for (const c of CLASSIFIERS) {
      if (c.types.includes(meta.type)) return c.field;
    }
    const haystack = `${meta.name} ${meta.id} ${meta.placeholder} ${meta.label}`;
    for (const c of CLASSIFIERS) {
      if (c.pattern.test(haystack)) return c.field;
    }
    return null;
  }

  /// Returns candidate fills: one DOM field per package field, with the current value for undo.
  function planFill(doc, prefill) {
    const inputs = Array.from(doc.querySelectorAll("input, textarea"));
    const plan = [];
    const taken = new Set();
    for (const input of inputs) {
      const meta = describe(input, doc);
      if (!isFillable(meta, input)) continue;
      const field = classify(meta);
      if (!field || taken.has(field)) continue;
      if (!(field in prefill) || prefill[field] == null || prefill[field] === "") continue;
      taken.add(field);
      plan.push({
        field,
        value: String(prefill[field]),
        selector: selectorFor(input, doc),
        label: meta.label || meta.placeholder || meta.name || meta.id || "(unlabelled field)",
        currentValue: input.value || "",
      });
    }
    return plan;
  }

  function selectorFor(input, doc) {
    if (input.id) return `#${cssEscapeId(input.id)}`;
    const name = input.getAttribute("name");
    if (name) {
      const sel = `${input.tagName.toLowerCase()}[name="${cssEscape(name)}"]`;
      if (doc.querySelectorAll(sel).length === 1) return sel;
    }
    const all = Array.from(doc.querySelectorAll("input, textarea"));
    return `__index:${all.indexOf(input)}`;
  }

  function cssEscapeId(id) {
    return String(id).replace(/([^a-zA-Z0-9_-])/g, "\\$1");
  }

  function resolve(doc, selector) {
    if (selector.startsWith("__index:")) {
      const all = Array.from(doc.querySelectorAll("input, textarea"));
      return all[Number(selector.slice(8))] || null;
    }
    return doc.querySelector(selector);
  }

  /// Fills only the approved entries. Returns an undo record. Never submits.
  function applyFill(doc, plan, approvedFields) {
    const undo = [];
    for (const entry of plan) {
      if (!approvedFields.includes(entry.field)) continue;
      const el = resolve(doc, entry.selector);
      if (!el) continue;
      const meta = describe(el, doc);
      if (!isFillable(meta, el)) continue; // the page may have changed since the preview
      undo.push({ selector: entry.selector, previous: el.value || "" });
      setValue(el, entry.value);
    }
    return undo;
  }

  function undoFill(doc, undo) {
    let restored = 0;
    for (const u of undo) {
      const el = resolve(doc, u.selector);
      if (!el) continue;
      setValue(el, u.previous);
      restored += 1;
    }
    return restored;
  }

  function setValue(el, value) {
    el.value = value;
    const view = el.ownerDocument && el.ownerDocument.defaultView;
    const Ev = (view && view.Event) || (typeof Event !== "undefined" ? Event : null);
    if (Ev) {
      el.dispatchEvent(new Ev("input", { bubbles: true }));
      el.dispatchEvent(new Ev("change", { bubbles: true }));
    }
  }

  return { planFill, applyFill, undoFill, classify, isFillable, describe, BLOCKED };
});
