//! End-to-end flows through `Service::execute`, the path every surface uses.

use openclaim_core::{Paths, Request, Service};
use openclaim_domain::ErrorCode;
use serde_json::{Value, json};

const LISTING: &str = include_str!("../../../test-fixtures/html/classaction_org_settlements.html");
const LISTING_URL: &str = "https://www.classaction.org/settlements";
const OFFICIAL_URL: &str = "https://www.personalcareproductssettlement.com/";

/// A synthetic copy of the Dr. Squatch official settlement page.
fn official_page(deadline_sentence: &str) -> String {
    format!(
        "<html><body><h1>Dr. Squatch All Natural Claims Settlement</h1><p>If you bought certain Dr. Squatch products between November 1, 2018 and August 29, 2026 you may be a class member.</p><p>{deadline_sentence}</p></body></html>"
    )
}

struct Harness {
    svc: Service,
    session: String,
    _dir: tempfile::TempDir,
}

fn harness() -> Harness {
    let dir = tempfile::tempdir().unwrap();
    let mut paths = Paths::from_dir(dir.path().to_path_buf());
    paths.in_memory = true;
    let svc = Service::open(paths).unwrap();
    let init = svc.execute(Request::new("owner.init", json!({"passphrase": "correct horse battery"})));
    assert!(init.ok, "{init:?}");
    let unlock = svc.execute(Request::new("owner.unlock", json!({"passphrase": "correct horse battery"})));
    assert!(unlock.ok, "{unlock:?}");
    let session = unlock.data.unwrap()["session"].as_str().unwrap().to_string();
    Harness { svc, session, _dir: dir }
}

impl Harness {
    fn owner(&self, op: &str, params: Value) -> Value {
        let env = self.svc.execute(Request::new(op, params).with_session(&self.session));
        assert!(env.ok, "{op} failed: {:?}", env.error);
        env.data.expect("data")
    }

    fn owner_err(&self, op: &str, params: Value) -> openclaim_domain::OcError {
        let env = self.svc.execute(Request::new(op, params).with_session(&self.session));
        let data = env.data.clone();
        assert!(!env.ok, "{op} unexpectedly succeeded: {data:?}");
        env.error.expect("error")
    }

    fn register_agent(&self, name: &str, scopes: &[&str]) -> String {
        let d = self.owner("agents.register", json!({"name": name, "scopes": scopes}));
        d["token"].as_str().unwrap().to_string()
    }

    fn agent(&self, token: &str, op: &str, params: Value) -> openclaim_domain::Envelope {
        let mut req = Request::new(op, params).with_agent_token(token);
        req.non_interactive = true;
        self.svc.execute(req)
    }

    fn import_listing(&self) {
        let r = self.owner(
            "sources.import_snapshot",
            json!({"source_id": "classaction-org", "url": LISTING_URL, "html": LISTING}),
        );
        assert_eq!(r["parsed"], 8);
    }

    fn squatch_id(&self) -> String {
        let r = self.owner("opportunities.list", json!({"q": "squatch"}));
        r["items"][0]["id"].as_str().unwrap().to_string()
    }

    /// Verifies the Dr. Squatch listing against a saved official page carrying `deadline_sentence`.
    fn confirm_squatch(&self, deadline_sentence: &str) -> Value {
        let id = self.squatch_id();
        self.owner(
            "opportunities.confirm_official",
            json!({"id": id, "url": OFFICIAL_URL, "html": official_page(deadline_sentence)}),
        )
    }
}

#[test]
fn anonymous_can_research_but_not_mutate() {
    let h = harness();
    let env = h.svc.execute(Request::new("opportunities.list", json!({})));
    assert!(env.ok);
    let env = h.svc.execute(Request::new("people.create", json!({"display_name": "X"})));
    assert_eq!(env.exit_code(), 4);
    assert_eq!(env.error.unwrap().code, ErrorCode::PermissionDenied);
    let env = h.svc.execute(Request::new("people.list", json!({})));
    assert_eq!(env.error.unwrap().code, ErrorCode::PermissionDenied);
}

#[test]
fn unknown_op_and_bad_passphrase() {
    let h = harness();
    let env = h.svc.execute(Request::new("nope.nothing", json!({})));
    assert_eq!(env.error.unwrap().code, ErrorCode::InvalidArgument);
    let env = h.svc.execute(Request::new("owner.unlock", json!({"passphrase": "wrong"})));
    assert_eq!(env.error.unwrap().code, ErrorCode::PermissionDenied);
    let env = h.svc.execute(Request::new("people.list", json!({})).with_session("ocs_bogus"));
    assert_eq!(env.error.unwrap().code, ErrorCode::PermissionDenied);
}

#[test]
fn snapshot_import_ingests_dedupes_and_verifies() {
    let h = harness();
    h.import_listing();
    let list = h.owner("opportunities.list", json!({"q": "squatch", "include": []}));
    assert!(list["pagination"].is_null(), "pagination is lifted into the envelope");
    let w = &list["items"][0];
    assert_eq!(w["verification_state"], "verification_pending", "a third-party listing is a lead");
    assert_eq!(w["status"], "open");
    assert_eq!(w["deadline_verified"], false);
    assert_eq!(w["trust"]["primary_source"], false);
    assert_eq!(w["claim_url"], OFFICIAL_URL);
    let id = w["id"].as_str().unwrap().to_string();
    // Re-importing the same listing must not duplicate.
    let total = h.owner("opportunities.list", json!({"limit": 100}))["items"].as_array().unwrap().len();
    h.owner(
        "sources.import_snapshot",
        json!({"source_id": "classaction-org", "url": LISTING_URL, "html": LISTING}),
    );
    assert_eq!(h.owner("opportunities.list", json!({"limit": 100}))["items"].as_array().unwrap().len(), total);
    let show = h.owner("opportunities.show", json!({"id": id, "include": ["provenance", "rules"]}));
    assert!(show["provenance"]["evidence"].as_array().unwrap().len() >= 3);
    assert_eq!(show["provenance"]["evidence"][0]["excerpt"]["trust"], "untrusted_source_text");
    assert!(show["rules"]["rules"].is_null());
    // The official site must name the settlement; an unrelated page is not a confirmation.
    let no = h.owner(
        "opportunities.confirm_official",
        json!({"id": id, "url": OFFICIAL_URL, "html": "<html><body><h1>Welcome</h1></body></html>"}),
    );
    assert_eq!(no["confirmed"], false);
    assert_eq!(
        h.owner("opportunities.show", json!({"id": id}))["opportunity"]["verification_state"],
        "verification_pending"
    );
    // A page on another domain is refused outright.
    let e = h.owner_err(
        "opportunities.confirm_official",
        json!({"id": id, "url": "https://evil.example/", "html": official_page("")}),
    );
    assert_eq!(e.code, ErrorCode::PolicyBlocked);
    // The real official page confirms it and carries the verified deadline.
    let yes = h.confirm_squatch("Claims must be submitted online by March 3, 2027.");
    assert_eq!(yes["confirmed"], true);
    assert_eq!(yes["deadline"], "2027-03-03");
    let w = h.owner("opportunities.show", json!({"id": id}))["opportunity"].clone();
    assert_eq!(w["verification_state"], "verified");
    assert_eq!(w["claim_deadline"], "2027-03-03");
    assert_eq!(w["deadline_verified"], true);
    let changes = h.owner("opportunities.changes", json!({"id": id}));
    assert!(changes["changes"].as_array().unwrap().iter().any(|c| c["field"] == "claim_deadline"));
}

#[test]
fn filter_dsl_and_pagination_cursor() {
    let h = harness();
    h.import_listing();
    let page1 = h.svc.execute(Request::new("opportunities.list", json!({"filter": "sort=title", "limit": 5})));
    assert!(page1.ok);
    let p = page1.pagination.clone().unwrap();
    assert_eq!(p.limit, 5);
    let cursor = p.next_cursor.clone().expect("more pages");
    let page2 = h.svc.execute(Request::new(
        "opportunities.list",
        json!({"filter": "sort=title", "limit": 5, "cursor": cursor}),
    ));
    assert!(page2.ok);
    let t1 = page1.data.unwrap()["items"][0]["title"].clone();
    let t2 = page2.data.unwrap()["items"][0]["title"].clone();
    assert_ne!(t1, t2);
    let bad = h.svc.execute(Request::new("opportunities.list", json!({"filter": "bogus=1"})));
    assert_eq!(bad.error.unwrap().code, ErrorCode::InvalidArgument);
    let verified_only = h
        .svc
        .execute(Request::new("opportunities.list", json!({"filter": "only_verified"})))
        .data
        .unwrap();
    assert!(verified_only["items"].as_array().unwrap().iter().all(|i| i["verification_state"] == "verified"));
}

#[test]
fn dry_run_never_commits() {
    let h = harness();
    let mut req = Request::new("people.create", json!({"display_name": "Dry"})).with_session(&h.session);
    req.dry_run = true;
    let env = h.svc.execute(req);
    assert!(env.ok && env.dry_run);
    assert!(env.warnings.iter().any(|w| w.contains("dry run")));
    let people = h.owner("people.list", json!({}));
    assert_eq!(people["items"].as_array().unwrap().len(), 0);
    let mut req = Request::new("sources.refresh", json!({"source_id": "classaction-org", "force": true})).with_session(&h.session);
    req.dry_run = true;
    let env = h.svc.execute(req);
    assert!(env.ok, "{env:?}");
    assert!(env.warnings.iter().any(|w| w.contains("network fetch")));
    let jobs = h.owner("jobs.list", json!({}));
    assert!(jobs["items"].as_array().unwrap().iter().all(|j| j["kind"] != "source_refresh"));
}

#[test]
fn idempotency_keys_replay_and_conflict() {
    let h = harness();
    let mut req = Request::new("people.create", json!({"display_name": "Idem"})).with_session(&h.session);
    req.idempotency_key = Some("k1".into());
    let a = h.svc.execute(req.clone());
    let b = h.svc.execute(req.clone());
    assert_eq!(a.data.unwrap()["id"], b.data.unwrap()["id"]);
    assert!(b.warnings.iter().any(|w| w.contains("replayed")));
    req.params = json!({"display_name": "Different"});
    let c = h.svc.execute(req);
    assert_eq!(c.error.unwrap().code, ErrorCode::Conflict);
    assert_eq!(h.owner("people.list", json!({}))["items"].as_array().unwrap().len(), 1);
}

#[test]
fn agent_scopes_and_approval_lifecycle() {
    let h = harness();
    let person = h.owner("people.create", json!({"display_name": "Ann"}))["id"].as_str().unwrap().to_string();
    let token = h.register_agent("researcher", &["discover.read", "opportunities.read", "sources.read", "jobs.read"]);
    // Read with scope works; reading personal data without scope is denied.
    assert!(h.agent(&token, "opportunities.list", json!({})).ok);
    let e = h.agent(&token, "people.show", json!({"id": person})).error.unwrap();
    assert_eq!(e.code, ErrorCode::PermissionDenied);
    assert_eq!(e.needed_scopes, vec!["people.read"]);
    // Owner-only operations can never be approved for an agent.
    assert_eq!(
        h.agent(&token, "agents.register", json!({"name": "self", "scopes": ["people.modify"]}))
            .error
            .unwrap()
            .code,
        ErrorCode::PermissionDenied
    );
    assert_eq!(
        h.agent(&token, "data.wipe", json!({"confirm": "WIPE"})).error.unwrap().code,
        ErrorCode::PermissionDenied
    );
    assert_eq!(
        h.agent(&token, "approvals.approve", json!({"id": "x"})).error.unwrap().code,
        ErrorCode::PermissionDenied
    );
    // A mutation without scope yields APPROVAL_REQUIRED (exit 5) with an approval id, in non-interactive mode.
    let env = h.agent(&token, "opportunities.bookmark", json!({"id": "nothing"}));
    let e = env.error.clone().unwrap();
    assert_eq!(e.code, ErrorCode::ApprovalRequired);
    assert_eq!(env.exit_code(), 5);
    let approval_id = e.approval_id.unwrap();
    // Approval is visible to the owner with a redacted preview; the agent can poll it.
    let shown = h.owner("approvals.show", json!({"id": approval_id}));
    assert_eq!(shown["decision"], "pending");
    assert_eq!(
        h.agent(&token, "approvals.show", json!({"id": approval_id})).data.unwrap()["decision"],
        "pending"
    );
    // Deny → terminal.
    h.owner("approvals.deny", json!({"id": approval_id}));
    let env = h.agent(&token, "opportunities.bookmark", json!({"id": "nothing"}));
    let e2 = env.error.unwrap();
    assert_eq!(e2.code, ErrorCode::ApprovalRequired);
    assert_ne!(e2.approval_id.unwrap(), approval_id, "a denied approval is not reused");
}

#[test]
fn approved_action_runs_once_and_is_digest_bound() {
    let h = harness();
    h.import_listing();
    let opp = h.squatch_id();
    let token = h.register_agent("curator", &["opportunities.read", "opportunities.write"]);
    // Elevated op with scope still needs approval.
    let env = h.agent(&token, "opportunities.verify", json!({"id": opp, "state": "disputed", "note": "looks stale"}));
    let aid = env.error.unwrap().approval_id.unwrap();
    h.owner("approvals.approve", json!({"id": aid}));
    let mut req = Request::new("opportunities.verify", json!({"id": opp, "state": "disputed", "note": "looks stale"})).with_agent_token(&token);
    req.approval_id = Some(aid.clone());
    let ok = h.svc.execute(req.clone());
    assert!(ok.ok, "{ok:?}");
    assert!(ok.audit_event_id.is_some());
    // Consumed: second use is refused.
    let again = h.svc.execute(req.clone());
    assert_eq!(again.error.unwrap().code, ErrorCode::PermissionDenied);
    // Different params → digest mismatch → conflict/invalidated.
    let env = h.agent(&token, "opportunities.verify", json!({"id": opp, "state": "verified", "note": "x"}));
    let aid2 = env
        .error
        .clone()
        .unwrap_or_else(|| panic!("expected approval: {env:?}"))
        .approval_id
        .unwrap_or_else(|| panic!("no approval id: {env:?}"));
    h.owner("approvals.approve", json!({"id": aid2}));
    h.owner("opportunities.verify", json!({"id": opp, "state": "verified", "note": "owner bumped version"}));
    let mut req2 = Request::new("opportunities.verify", json!({"id": opp, "state": "verified", "note": "x"})).with_agent_token(&token);
    req2.approval_id = Some(aid2);
    let env = h.svc.execute(req2);
    assert_eq!(env.error.unwrap().code, ErrorCode::Conflict, "record changed after approval");
}

#[test]
fn revoked_agent_is_locked_out() {
    let h = harness();
    let token = h.register_agent("temp", &["discover.read"]);
    let id = h.owner("agents.list", json!({}))["items"][0]["id"].as_str().unwrap().to_string();
    assert!(h.agent(&token, "discover.search", json!({})).ok);
    assert_eq!(
        h.agent(&token, "opportunities.list", json!({})).error.unwrap().code,
        ErrorCode::PermissionDenied,
        "discover.read does not imply opportunities.read"
    );
    h.owner("agents.revoke", json!({"id": id}));
    assert_eq!(h.agent(&token, "discover.search", json!({})).error.unwrap().code, ErrorCode::PermissionDenied);
    let audit = h.owner("agents.audit", json!({"id": id}));
    assert!(audit["commands"].as_array().unwrap().len() >= 2);
}

#[tokio::test]
async fn matching_claims_and_alerts_flow() {
    let h = harness();
    h.import_listing();
    let opp = h.squatch_id();
    // Verify against the official site so the deadline counts.
    assert_eq!(h.confirm_squatch("You must file a claim by December 1, 2099.")["confirmed"], true);
    let person = h.owner(
        "people.create",
        json!({"display_name": "Ann", "details": {"emails": ["ann@example.test"], "mailing_address": "1 Main St"}, "consent": {"authorized_by_adult": true}}),
    )["id"]
        .as_str()
        .unwrap()
        .to_string();
    h.owner(
        "people.add_residence",
        json!({"person_id": person, "jurisdiction": "TX", "start": "2010-01-01"}),
    );
    // No rules yet → manual review.
    let m = h.owner("match.evaluate", json!({"person_id": person, "opportunity_id": opp}));
    assert_eq!(m["assessment"]["state"], "manual_review");
    // Curate rules from the official text.
    let rules = json!({"rules": [
        {"id": "purchase", "label": "Bought a Dr. Squatch product in the class period", "criterion": {"kind": "consumer_fact", "fact_type": "purchase", "merchant": "Dr. Squatch", "during": {"start": "2018-11-01", "end": "2026-08-29"}}, "source_excerpt": "if you bought certain Dr. Squatch products between November 1, 2018 and August 29, 2026", "source_locator": "https://www.personalcareproductssettlement.com/#class"},
        {"id": "us", "label": "US resident", "criterion": {"kind": "resided_in", "jurisdictions": ["US", "TX", "CA"], "during": null}, "source_excerpt": "United States residents", "source_locator": "https://www.personalcareproductssettlement.com/#class"}
    ], "one_claim_per": "person"});
    h.owner("opportunities.set_rules", json!({"opportunity_id": opp, "rules": rules, "original_text_excerpt": "You may be able to claim a piece of this settlement if you bought certain Dr. Squatch products between November 1, 2018 and August 29, 2026."}));
    let m = h.owner("match.evaluate", json!({"person_id": person, "opportunity_id": opp}));
    assert_eq!(m["assessment"]["state"], "needs_more_information");
    let missing = h.owner("match.missing", json!({"person_id": person}));
    assert_eq!(missing["items"].as_array().unwrap().len(), 1);
    // Agent proposes a fact; it is not used until confirmed.
    let token = h.register_agent("helper", &["facts.propose", "match.evaluate", "people.read"]);
    let prop = h
        .agent(
            &token,
            "facts.propose",
            json!({"person_id": person, "proposal": {"fact_type": "purchase", "merchant": "Dr. Squatch", "start": "2020-05-05", "end": "2020-05-05"}}),
        )
        .data
        .unwrap();
    assert_eq!(prop["trust"], "agent_proposed");
    let m = h.owner("match.evaluate", json!({"person_id": person, "opportunity_id": opp}));
    assert_eq!(m["assessment"]["state"], "needs_more_information");
    assert!(
        h.agent(
            &token,
            "facts.add",
            json!({"person_id": person, "fact_type": "purchase", "merchant": "Dr. Squatch"})
        )
        .error
        .is_some()
    );
    h.owner("facts.confirm", json!({"id": prop["proposal_id"]}));
    let m = h.owner("match.evaluate", json!({"person_id": person, "opportunity_id": opp}));
    assert_eq!(m["assessment"]["state"], "potential_match", "{m}");
    assert!(m["assessment"]["disclaimer"].as_str().unwrap().contains("Only the official administrator"));
    // Agent-safe explanation hides personal explanations without people.read.
    let token2 = h.register_agent("readonly", &["match.evaluate"]);
    let ex = h
        .agent(&token2, "match.explain", json!({"person_id": person, "opportunity_id": opp}))
        .data
        .unwrap();
    assert!(ex["assessment"]["redacted"].is_string());
    // Claim lifecycle.
    let claim = h.owner("claims.init", json!({"person_id": person, "opportunity_id": opp}))["id"]
        .as_str()
        .unwrap()
        .to_string();
    let check = h.owner("claims.checklist", json!({"id": claim}));
    assert_eq!(check["ready"], true, "{check}");
    let prep = h.owner(
        "claims.prepare",
        json!({"claim_id": claim, "prefill_fields": ["name", "email", "mailing_address"]}),
    );
    assert_eq!(prep["status"], "ready");
    assert!(
        h.owner_err("claims.prepare", json!({"claim_id": claim, "prefill_fields": ["ssn"]}))
            .message
            .contains("cannot be prefilled")
    );
    let hand = h.owner("claims.handoff", json!({"claim_id": claim}));
    assert_eq!(hand["open_url"], OFFICIAL_URL);
    // Extension principal gets prefill only for the allowed domain.
    let ext = h.owner("agents.register", json!({"name": "ext", "kind": "extension", "scopes": ["extension.prefill"]}))["token"]
        .as_str()
        .unwrap()
        .to_string();
    let pkg = prep["package_id"].as_str().unwrap();
    let bad = h.agent(&ext, "claims.prefill_package", json!({"package_id": pkg, "origin": "https://evil.example"}));
    assert_eq!(bad.error.unwrap().code, ErrorCode::PolicyBlocked);
    let good = h
        .agent(
            &ext,
            "claims.prefill_package",
            json!({"package_id": pkg, "origin": "https://www.personalcareproductssettlement.com"}),
        )
        .data
        .unwrap();
    assert_eq!(good["prefill"]["email"], "ann@example.test");
    assert_eq!(good["never_submit"], true);
    // Submission requires confirmation + evidence; agents cannot record it.
    assert!(
        h.agent(
            &token,
            "claims.record_submission",
            json!({"claim_id": claim, "user_confirmed": true, "confirmation": "X"})
        )
        .error
        .is_some()
    );
    assert_eq!(
        h.owner_err(
            "claims.record_submission",
            json!({"claim_id": claim, "user_confirmed": false, "confirmation": "ABC"})
        )
        .code,
        ErrorCode::InvalidArgument
    );
    h.owner(
        "claims.record_submission",
        json!({"claim_id": claim, "user_confirmed": true, "confirmation": "ABC-123"}),
    );
    assert_eq!(
        h.owner_err("claims.update", json!({"claim_id": claim, "status": "paid", "user_confirmed": true}))
            .code,
        ErrorCode::InvalidArgument
    );
    h.owner(
        "claims.update",
        json!({"claim_id": claim, "status": "approved", "user_confirmed": true, "note": "letter received"}),
    );
    let paid = h.owner(
        "claims.update",
        json!({"claim_id": claim, "status": "paid", "user_confirmed": true, "amount_minor": 12345, "currency": "usd"}),
    );
    assert_eq!(paid["status"], "paid");
    let ledger = h.owner("claims.ledger", json!({}));
    assert_eq!(ledger["totals"][0]["total_minor"], 12345);
    // Alerts exist for the verified future deadline.
    let alerts = h.owner("alerts.list", json!({"due_within_days": 40000, "include_dismissed": true}));
    assert!(alerts["items"].as_array().unwrap().iter().any(|a| a["kind"] == "deadline"), "{alerts}");
    let ics = h.owner("alerts.export_ics", json!({}));
    assert!(ics["ics"].as_str().unwrap().contains("BEGIN:VCALENDAR"));
    // Export round-trips.
    let export = h.owner("data.export", json!({}));
    assert_eq!(export["people"][0]["details"]["emails"][0], "ann@example.test");
    assert_eq!(export["claims"][0]["status"], "paid");
}

#[tokio::test]
async fn jobs_and_document_extraction() {
    let h = harness();
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("receipt.txt");
    std::fs::write(
        &file,
        "Receipt\nMerchant: Acme Widgets\nOrder date: March 3, 2020\nTotal: $12.50\nCard 4111 1111 1111 1111",
    )
    .unwrap();
    let person = h.owner("people.create", json!({"display_name": "Doc"}))["id"].as_str().unwrap().to_string();
    let imp = h.owner("documents.import", json!({"file": file.to_string_lossy(), "person_id": person}));
    let doc = imp["id"].as_str().unwrap().to_string();
    assert_eq!(imp["duplicate"], false);
    let dup = h.owner("documents.import", json!({"file": file.to_string_lossy()}));
    assert_eq!(dup["duplicate"], true);
    assert!(h.svc.drain_jobs().await >= 1);
    let shown = h.owner("documents.show", json!({"id": doc}));
    assert_eq!(shown["extraction_state"], "extracted");
    assert_eq!(shown["classification"], "receipt");
    let sugg = h.owner("documents.suggestions", json!({"id": doc}));
    assert!(
        sugg["suggestions"]
            .as_array()
            .unwrap()
            .iter()
            .any(|s| s["kind"] == "merchant" && s["value"] == "Acme Widgets")
    );
    let ex = h.owner("documents.excerpt", json!({"id": doc, "offset": 0, "length": 500}));
    let text = ex["text"]["text"].as_str().unwrap();
    assert!(text.contains("Acme Widgets"));
    assert!(!text.contains("4111"), "card numbers are redacted before indexing");
    let search = h.owner("documents.search", json!({"q": "widgets"}));
    assert_eq!(search["items"].as_array().unwrap().len(), 1);
    // Agent with metadata scope but no read scope sees no text.
    let token = h.register_agent("reader", &["documents.metadata"]);
    assert!(h.agent(&token, "documents.list", json!({})).ok);
    assert_eq!(
        h.agent(&token, "documents.excerpt", json!({"id": doc})).error.unwrap().code,
        ErrorCode::PermissionDenied
    );
    // Narrowed grant for a different document does not unlock this one.
    let token2 = h.register_agent("narrow", &["documents.read:other-doc"]);
    assert_eq!(
        h.agent(&token2, "documents.excerpt", json!({"id": doc})).error.unwrap().code,
        ErrorCode::PermissionDenied
    );
    let token3 = h.register_agent("exact", &[&format!("documents.read:{doc}")]);
    assert!(h.agent(&token3, "documents.excerpt", json!({"id": doc})).ok);
    // Jobs are visible with events; the integrity job runs.
    let jobs = h.owner("jobs.list", json!({}));
    assert!(
        jobs["items"]
            .as_array()
            .unwrap()
            .iter()
            .any(|j| j["kind"] == "document_extract" && j["state"] == "succeeded")
    );
    h.owner("jobs.enqueue", json!({"kind": "integrity_check"}));
    h.svc.drain_jobs().await;
    let integ = h.owner("data.integrity", json!({}));
    assert_eq!(integ["ok"], true, "{integ}");
    let verify = h.owner("audit.verify", json!({}));
    assert_eq!(verify["intact"], true);
}

#[tokio::test]
async fn workflows_pause_for_approval_and_resume() {
    let h = harness();
    h.import_listing();
    let def = json!({
        "schema_version": 1,
        "name": "mark-squatch",
        "permissions": ["opportunities.read", "opportunities.write"],
        "steps": [
            {"id": "find", "operation": "opportunities.list", "with": {"q": "squatch"}},
            {"id": "mark", "operation": "opportunities.bookmark", "with": {"id": "${steps.find.data.items.0.id}"}, "after": ["find"]},
            {"id": "never", "operation": "opportunities.hide", "with": {"id": "x"}, "when": {"step": "find", "path": "items.0.title", "equals": "no such"}}
        ]
    });
    let bad = h.owner(
        "workflows.validate",
        json!({"definition": {"schema_version": 1, "name": "bad", "permissions": ["agents.manage"], "steps": [{"id": "a", "operation": "agents.register"}]}}),
    );
    assert_eq!(bad["valid"], false);
    let plan = h.owner("workflows.plan", json!({"definition": def}));
    assert_eq!(plan["order"], json!(["find", "mark", "never"]));
    h.owner("workflows.register", json!({"definition": def}));
    // Owner run completes end to end.
    let run = h.owner("workflows.run", json!({"name": "mark-squatch"}));
    h.svc.drain_jobs().await;
    let st = h.owner("workflows.status", json!({"run_id": run["run_id"]}));
    assert_eq!(st["state"], "succeeded", "{st}");
    assert_eq!(st["checkpoint"]["skipped"], json!(["never"]));
    let w = h.owner("opportunities.list", json!({"q": "squatch"}));
    assert_eq!(w["items"][0]["bookmarked"], true);
    // Agent run pauses for approval on the write step, then resumes after approval.
    let token = h.register_agent("flow", &["opportunities.read", "opportunities.write", "workflows.run", "jobs.read"]);
    let run = h.agent(&token, "workflows.run", json!({"name": "mark-squatch"})).data.unwrap();
    h.svc.drain_jobs().await;
    let st = h.agent(&token, "workflows.status", json!({"run_id": run["run_id"]})).data.unwrap();
    assert_eq!(st["state"], "succeeded", "write with scope needs no approval: {st}");
    let def2 = json!({"schema_version": 1, "name": "gated", "permissions": ["opportunities.read", "opportunities.write"], "steps": [{"id": "mark", "operation": "opportunities.hide", "with": {"id": h.squatch_id()}, "approval": true}]});
    h.owner("workflows.register", json!({"definition": def2}));
    let run = h.agent(&token, "workflows.run", json!({"name": "gated"})).data.unwrap();
    h.svc.drain_jobs().await;
    let st = h.agent(&token, "workflows.status", json!({"run_id": run["run_id"]})).data.unwrap();
    assert_eq!(st["state"], "paused_for_approval", "{st}");
    let pending = h.owner("approvals.list", json!({"state": "pending"}));
    let aid = pending["items"][0]["id"].as_str().unwrap();
    h.owner("approvals.approve", json!({"id": aid}));
    h.svc.drain_jobs().await;
    let st = h.agent(&token, "workflows.status", json!({"run_id": run["run_id"]})).data.unwrap();
    assert_eq!(st["state"], "succeeded", "{st}");
}

#[test]
fn schema_export_lists_every_operation_with_mcp_tools() {
    let h = harness();
    let s = h.svc.execute(Request::new("schema.export", json!({}))).data.unwrap();
    let ops = s["operations"].as_array().unwrap();
    assert!(ops.len() > 100, "{}", ops.len());
    for name in [
        "discover_opportunities",
        "get_opportunity",
        "get_opportunity_rules",
        "compare_opportunities",
        "evaluate_match",
        "list_document_metadata",
        "read_document_excerpt",
        "propose_fact",
        "get_claim_checklist",
        "prepare_claim_draft",
        "get_claim_status",
        "refresh_source",
        "get_job",
        "watch_job",
        "list_alerts",
        "request_approval",
        "get_approval_status",
        "get_source_health",
    ] {
        assert!(ops.iter().any(|o| o["mcp_tool"] == name), "missing MCP tool {name}");
    }
    assert!(!ops.iter().any(|o| o["name"].as_str().unwrap().contains("submit_official")));
    let v = h
        .svc
        .execute(Request::new("schema.validate", json!({"op": "opportunities.show", "params": {"include": "x"}})))
        .data
        .unwrap();
    assert_eq!(v["valid"], false);
}

#[test]
fn wipe_and_restore_cycle() {
    let h = harness();
    h.owner("people.create", json!({"display_name": "Gone"}));
    let dir = tempfile::tempdir().unwrap();
    let backup = dir.path().join("b.sqlite");
    h.owner("data.backup", json!({"path": backup.to_string_lossy()}));
    assert_eq!(h.owner_err("data.wipe", json!({"confirm": "no"})).code, ErrorCode::InvalidArgument);
    h.owner("data.wipe", json!({"confirm": "WIPE"}));
    // Wiping ends owner sessions; unlock again.
    let unlock = h.svc.execute(Request::new("owner.unlock", json!({"passphrase": "correct horse battery"})));
    let h = Harness {
        session: unlock.data.unwrap()["session"].as_str().unwrap().to_string(),
        ..h
    };
    assert_eq!(h.owner("people.list", json!({}))["items"].as_array().unwrap().len(), 0);
    let r = h.owner("data.restore", json!({"path": backup.to_string_lossy(), "confirm": true}));
    assert_eq!(r["schema_version"], "1");
    assert_eq!(h.owner("people.list", json!({}))["items"].as_array().unwrap().len(), 1);
}

#[tokio::test]
async fn failed_refresh_is_recorded_in_source_health() {
    let h = harness();
    h.owner("config.set", json!({"key": "network.offline", "value": true}));
    // Bypass the offline guard in sources.refresh by enqueueing the job directly, as the scheduler would.
    h.owner(
        "jobs.enqueue",
        json!({"kind": "source_refresh", "params": {"source_id": "classaction-org", "force": true}}),
    );
    h.svc.drain_jobs().await;
    let health = h.owner("sources.health", json!({}));
    let src = health["items"].as_array().unwrap().iter().find(|s| s["id"] == "classaction-org").unwrap();
    assert_eq!(src["last_failure"], "offline", "{src}");
    assert_eq!(src["health"], "degraded");
    let jobs = h.owner("jobs.list", json!({"kind": "source_refresh"}));
    assert_eq!(jobs["items"][0]["state"], "failed");
    assert_eq!(jobs["items"][0]["error_code"], "POLICY_BLOCKED");
}
