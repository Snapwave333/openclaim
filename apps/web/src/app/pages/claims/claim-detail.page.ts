import { ChangeDetectionStrategy, Component, OnInit, inject, input, signal } from '@angular/core';
import { RouterLink } from '@angular/router';
import { ApiService } from '../../core/api.service';
import { Checklist, ClaimDetail, Doc, Handoff, Paged } from '../../core/api.types';
import { date, dateOnly, humanize, money } from '../../core/format';
import { ToastService } from '../../core/toast.service';
import { BadgeComponent } from '../../shared/badge.component';
import { TermComponent } from '../../shared/term.component';

const PREFILL = ['name', 'email', 'phone', 'mailing_address'];

@Component({
  selector: 'oc-claim-detail',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [RouterLink, BadgeComponent, TermComponent],
  template: `
    <p><a routerLink="/claims">← Back to Claims</a></p>
    @if (detail(); as d) {
      <header class="page-header">
        <div>
          <h1>{{ d.claim.opportunity_title }}</h1>
          <div class="opp__meta">
            <span>For {{ d.claim.display_name }}</span>
            <span>· <oc-badge [value]="d.claim.status" /></span>
            <span>· <oc-badge [value]="d.claim.verification_state" /></span>
            <span>· <oc-term key="claim_deadline">Deadline</oc-term> {{ d.claim.claim_deadline ? dateOnly(d.claim.claim_deadline) + (d.claim.deadline_verified ? ' (verified)' : ' (not verified)') : 'unknown' }}</span>
          </div>
        </div>
        <div class="row">
          <a class="btn" [routerLink]="['/discover', d.claim.opportunity_id]">Opportunity</a>
          <a class="btn" [href]="d.claim.official_url" target="_blank" rel="noopener noreferrer">View original source ↗</a>
        </div>
      </header>

      <div class="grid grid--2">
        <section class="card" aria-labelledby="cl-h">
          <h2 id="cl-h">Checklist</h2>
          @if (checklist(); as c) {
            <p>Ready to file: <strong>{{ c.ready ? 'yes' : 'not yet' }}</strong> · Eligibility: <oc-badge [value]="c.eligibility.state" /></p>
            <p class="small muted">{{ c.eligibility.summary }}</p>
            @if (c.warnings.length) {
              <ul class="small" style="color: var(--warn)">@for (w of c.warnings; track w) { <li>{{ w }}</li> }</ul>
            }
            <h3>Next steps</h3>
            <ol>@for (s of c.next_steps; track s) { <li>{{ s }}</li> }</ol>
            <p class="small muted">Linked documents: {{ c.linked_documents.length }} · Missing facts: {{ c.missing_facts.length }}</p>
          } @else { <p class="muted">Loading checklist…</p> }
        </section>

        <section class="card" aria-labelledby="pp-h">
          <h2 id="pp-h">Prepare package</h2>
          <p class="hint">Choose which profile fields a paired browser extension may preview for prefill. Nothing is sent anywhere; the package stays local and expires.</p>
          <fieldset>
            <legend>Prefill fields</legend>
            @for (f of prefillOptions; track f) {
              <label class="field field--inline"><input type="checkbox" [checked]="prefill().includes(f)" (change)="togglePrefill(f, $any($event.target).checked)" /> {{ humanize(f) }}</label>
            }
          </fieldset>
          <button class="btn btn--primary" type="button" (click)="prepare()" [disabled]="busy()">Prepare package</button>
          @if (packageId()) { <p class="small muted" style="margin-top:0.5rem">Package <code>{{ packageId() }}</code> prepared.</p> }
        </section>

        <section class="card" aria-labelledby="ho-h">
          <h2 id="ho-h"><oc-term key="handoff">Official filing handoff</oc-term></h2>
          <p class="notice">OpenClaim never submits claims. CAPTCHA, identity verification, signature and Submit stay with you.</p>
          @if (handoff(); as h) {
            <p><a class="btn btn--primary" [href]="h.open_url" target="_blank" rel="noopener noreferrer">Open official site ↗</a> <span class="small muted">{{ h.allowed_domain }}</span></p>
            <h3>You must complete</h3>
            <ol>@for (i of h.instructions; track i) { <li>{{ i }}</li> }</ol>
          } @else {
            <button class="btn" type="button" (click)="getHandoff()" [disabled]="busy()">Show handoff instructions</button>
          }
        </section>

        <section class="card" aria-labelledby="rs-h">
          <h2 id="rs-h">Record submission</h2>
          <p class="hint">Only record this after you submitted the claim yourself on the official site.</p>
          <form (submit)="recordSubmission($event)">
            <div class="field"><label for="conf">Confirmation id</label><input id="conf" type="text" [value]="confirmation()" (input)="confirmation.set($any($event.target).value)" required /></div>
            <div class="field"><label for="conf-doc">Evidence document (optional)</label>
              <select id="conf-doc" [value]="confirmationDoc()" (change)="confirmationDoc.set($any($event.target).value)"><option value="">None</option>@for (doc of docs(); track doc.id) { <option [value]="doc.id">{{ doc.name }}</option> }</select></div>
            <label class="field field--inline"><input type="checkbox" [checked]="reallySubmitted()" (change)="reallySubmitted.set($any($event.target).checked)" /> I really submitted this claim myself on the official site.</label>
            <button class="btn btn--primary" type="submit" [disabled]="!reallySubmitted() || !confirmation().trim() || busy()">Record submission</button>
          </form>
        </section>

        <section class="card" aria-labelledby="st-h">
          <h2 id="st-h">Update status</h2>
          <form (submit)="updateStatus($event)">
            <div class="field"><label for="st">New status</label>
              <select id="st" [value]="newStatus()" (change)="newStatus.set($any($event.target).value)">
                <option value="">Choose…</option>
                @for (s of d.claim.allowed_next; track s) { <option [value]="s">{{ humanize(s) }}</option> }
              </select></div>
            <div class="field"><label for="st-note">Note</label><input id="st-note" type="text" [value]="note()" (input)="note.set($any($event.target).value)" /></div>
            @if (newStatus() === 'paid') {
              <div class="field"><label for="st-amount">Amount received (e.g. 12.50)</label><input id="st-amount" type="text" inputmode="decimal" [value]="amount()" (input)="amount.set($any($event.target).value)" /></div>
            }
            <div class="field"><label for="st-doc">Evidence document (optional)</label>
              <select id="st-doc" [value]="statusDoc()" (change)="statusDoc.set($any($event.target).value)"><option value="">None</option>@for (doc of docs(); track doc.id) { <option [value]="doc.id">{{ doc.name }}</option> }</select></div>
            @if (newStatus() === 'submitted') {
              <label class="field field--inline"><input type="checkbox" [checked]="reallySubmitted()" (change)="reallySubmitted.set($any($event.target).checked)" /> I really submitted this claim myself.</label>
            }
            <button class="btn" type="submit" [disabled]="!newStatus() || busy() || (newStatus() === 'submitted' && !reallySubmitted())">Update</button>
          </form>
        </section>

        <section class="card" aria-labelledby="ev-h">
          <h2 id="ev-h">Timeline</h2>
          <ul class="plain">
            @for (e of d.events; track e.timestamp + e.event_type) {
              <li><strong>{{ humanize(e.event_type) }}</strong> @if (e.to_status) { → <oc-badge [value]="e.to_status" /> } <span class="small muted">· {{ date(e.timestamp) }} · {{ e.who_recorded }}</span>@if (e.evidence_id) { <span class="small faint"> · evidence {{ e.evidence_id.slice(0, 8) }}</span> }</li>
            }
          </ul>
          @if (d.claim.manual_paid_amount !== null) { <p>Paid: <strong>{{ money(d.claim.manual_paid_amount, d.claim.currency || 'USD') }}</strong></p> }
        </section>
      </div>
    } @else { <p class="muted" role="status">Loading…</p> }
  `,
})
export class ClaimDetailPage implements OnInit {
  private readonly api = inject(ApiService);
  private readonly toast = inject(ToastService);

  readonly id = input.required<string>();
  readonly detail = signal<ClaimDetail | null>(null);
  readonly checklist = signal<Checklist | null>(null);
  readonly handoff = signal<Handoff | null>(null);
  readonly docs = signal<Doc[]>([]);
  readonly busy = signal(false);
  readonly prefillOptions = PREFILL;
  readonly prefill = signal<string[]>(['name']);
  readonly packageId = signal<string | null>(null);
  readonly confirmation = signal('');
  readonly confirmationDoc = signal('');
  readonly reallySubmitted = signal(false);
  readonly newStatus = signal('');
  readonly note = signal('');
  readonly amount = signal('');
  readonly statusDoc = signal('');

  readonly humanize = humanize;
  readonly date = date;
  readonly dateOnly = dateOnly;
  readonly money = money;

  async ngOnInit(): Promise<void> {
    await this.reload();
    try { this.docs.set((await this.api.op<Paged<Doc>>('documents.list', { limit: 200 })).items ?? []); } catch { /* optional */ }
  }

  async reload(): Promise<void> {
    try {
      const [d, c] = await Promise.all([this.api.op<ClaimDetail>('claims.show', { id: this.id() }), this.api.op<Checklist>('claims.checklist', { id: this.id() })]);
      this.detail.set(d);
      this.checklist.set(c);
    } catch (e) { this.toast.error(e); }
  }

  togglePrefill(f: string, on: boolean): void { this.prefill.update((l) => (on ? [...new Set([...l, f])] : l.filter((x) => x !== f))); }

  private async run(fn: () => Promise<void>): Promise<void> {
    this.busy.set(true);
    try { await fn(); await this.reload(); } catch (e) { this.toast.error(e); } finally { this.busy.set(false); }
  }

  prepare(): Promise<void> {
    return this.run(async () => {
      const r = await this.api.op<{ package_id: string }>('claims.prepare', { claim_id: this.id(), prefill_fields: this.prefill(), expected_version: this.detail()?.claim.version ?? null });
      this.packageId.set(r.package_id);
      this.toast.ok('Package prepared locally.');
    });
  }

  getHandoff(): Promise<void> {
    return this.run(async () => { this.handoff.set(await this.api.op<Handoff>('claims.handoff', { claim_id: this.id() })); });
  }

  recordSubmission(ev: Event): Promise<void> {
    ev.preventDefault();
    return this.run(async () => {
      await this.api.op('claims.record_submission', { claim_id: this.id(), confirmation: this.confirmation().trim(), document_id: this.confirmationDoc() || null, user_confirmed: this.reallySubmitted(), expected_version: this.detail()?.claim.version ?? null });
      this.toast.ok('Submission recorded.');
      this.confirmation.set(''); this.reallySubmitted.set(false);
    });
  }

  updateStatus(ev: Event): Promise<void> {
    ev.preventDefault();
    return this.run(async () => {
      const amountMinor = this.amount().trim() ? Math.round(Number(this.amount()) * 100) : null;
      await this.api.op('claims.update', { claim_id: this.id(), status: this.newStatus(), user_confirmed: this.reallySubmitted(), note: this.note() || null, document_id: this.statusDoc() || null, amount_minor: amountMinor, currency: amountMinor !== null ? 'USD' : null, expected_version: this.detail()?.claim.version ?? null });
      this.toast.ok('Status updated.');
      this.newStatus.set(''); this.note.set(''); this.amount.set(''); this.statusDoc.set('');
    });
  }
}
