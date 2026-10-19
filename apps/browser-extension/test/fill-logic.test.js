const test = require("node:test");
const assert = require("node:assert/strict");
const { JSDOM } = require("jsdom");
const F = require("../src/fill-logic.js");

const PAGE = `
<form id="claim" action="/submit" method="post">
  <label for="fn">Full name</label><input id="fn" name="claimant_name">
  <label>Email address <input type="email" name="email"></label>
  <label for="ph">Phone</label><input id="ph" type="tel">
  <label for="addr">Mailing address line 1</label><input id="addr" name="address1" value="old street">
  <label for="ssn">Social Security Number</label><input id="ssn" name="ssn">
  <label for="card">Card number</label><input id="card" name="cc_number">
  <input type="password" name="password">
  <input id="sig" name="signature" placeholder="Type your name to sign">
  <input id="captcha" name="g-recaptcha-response">
  <input type="hidden" name="token" value="x">
  <input type="text" name="bank_account_number">
  <button type="submit" id="submit">Submit claim</button>
</form>`;

const PREFILL = { name: "Ann Example", email: "ann@example.test", phone: "555-0100", mailing_address: "1 Main St" };

function dom() {
  const d = new JSDOM(PAGE, { url: "https://claims.example.test/form" });
  return d.window.document;
}

test("plans only non-sensitive fields and reports current values", () => {
  const doc = dom();
  const plan = F.planFill(doc, PREFILL);
  const fields = plan.map((p) => p.field).sort();
  assert.deepEqual(fields, ["email", "mailing_address", "name", "phone"]);
  const addr = plan.find((p) => p.field === "mailing_address");
  assert.equal(addr.currentValue, "old street");
  assert.equal(addr.label, "Mailing address line 1");
  for (const p of plan) {
    assert.ok(!/ssn|card|password|signature|captcha|bank/.test(p.selector), p.selector);
  }
});

test("blocked fields are never fillable even when a value exists", () => {
  const doc = dom();
  for (const sel of ["#ssn", "#card", "input[name=password]", "#sig", "#captcha", "input[name=bank_account_number]", "input[name=token]"]) {
    const el = doc.querySelector(sel);
    assert.equal(F.isFillable(F.describe(el, doc), el), false, sel);
  }
});

test("apply respects the approved subset, fires events, and undo restores", () => {
  const doc = dom();
  const plan = F.planFill(doc, PREFILL);
  let inputEvents = 0;
  doc.querySelector("#fn").addEventListener("input", () => inputEvents++);
  const undo = F.applyFill(doc, plan, ["name", "mailing_address"]);
  assert.equal(doc.querySelector("#fn").value, "Ann Example");
  assert.equal(doc.querySelector("#addr").value, "1 Main St");
  assert.equal(doc.querySelector("input[name=email]").value, "", "unapproved field untouched");
  assert.equal(inputEvents, 1);
  assert.equal(undo.length, 2);
  const restored = F.undoFill(doc, undo);
  assert.equal(restored, 2);
  assert.equal(doc.querySelector("#fn").value, "");
  assert.equal(doc.querySelector("#addr").value, "old street");
});

test("no form is ever submitted by the fill logic", () => {
  const doc = dom();
  const form = doc.querySelector("#claim");
  let submitted = 0;
  form.addEventListener("submit", (e) => {
    submitted++;
    e.preventDefault();
  });
  form.submit = () => {
    submitted++;
  };
  form.requestSubmit = () => {
    submitted++;
  };
  const plan = F.planFill(doc, PREFILL);
  const undo = F.applyFill(doc, plan, plan.map((p) => p.field));
  F.undoFill(doc, undo);
  assert.equal(submitted, 0);
  const src = require("fs").readFileSync(require.resolve("../src/fill-logic.js"), "utf8") + require("fs").readFileSync(require.resolve("../src/content.js"), "utf8");
  assert.ok(!/\.submit\(|requestSubmit|\.click\(/.test(src), "fill code must not contain submit/click calls");
});

test("classification uses autocomplete before heuristics", () => {
  const doc = new JSDOM(`<input id="a" autocomplete="tel" name="contact"><input id="b" name="emailaddress">`, { url: "https://x.test/" }).window.document;
  assert.equal(F.classify(F.describe(doc.querySelector("#a"), doc)), "phone");
  assert.equal(F.classify(F.describe(doc.querySelector("#b"), doc)), "email");
});
