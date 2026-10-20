import { ChangeDetectionStrategy, Component, OnInit, computed, inject, input, signal } from '@angular/core';
import { Router, RouterLink } from '@angular/router';
import { ApiService } from '../../core/api.service';
import { ChangeItem, Claim, Handoff, MatchExplain, OpportunityDetail, Paged } from '../../core/api.types';
import { date, dateOnly, humanize, money } from '../../core/format';
import { ToastService } from '../../core/toast.service';
import { BadgeComponent } from '../../shared/badge.component';
import { PeopleService } from '../../shared/people.service';
import { PersonSelectComponent } from '../../shared/person-select.component';
import { TrustDisplayComponent } from '../../shared/trust-display.component';
import { UntrustedTextComponent } from '../../shared/untrusted-text.component';
import { MatchExplainComponent } from '../matches/match-explain.component';

type Tab = 'summary' | 'eligibility' | 'sources' | 'evidence' | 'filing' | 'tracking';
const TABS: Array<[Tab, string]> = [['summary', 'Summary'], ['eligibility', 'Eligibility'], ['sources', 'Supporting sources'], ['evidence', 'Required evidence'], ['filing', 'Official filing'], ['tracking', 'Tracking']];

@Component({
  selector: 'oc-opportunity-detail',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [RouterLink, BadgeComponent, TrustDisplayComponent, UntrustedTextComponent, PersonSelectComponent, MatchExplainComponent],
  template: `
    <p><a routerLink="/discover">← Back to Discover</a></p>
    @if (detail(); as d) {
      <header class="page-header">
        <div>
          <h1>{{ d.opportunity.title }}</h1>
          <div class="opp__meta">
            <oc-badge [value]="d.opportunity.status" />
            <span>{{ humanize(d.opportunity.opp_type) }}</span>
            @if (d.opportunity.administrator) { <span>· {{ d.opportunity.administrator }}</span> }
            @if (d.opportunity.case_number) { <span>· Case {{ d.opportunity.case_number }}</span> }
            <span>· Source: {{ d.opportunity.source_name || d.opportunity.source_id }} (tier {{ d.opportunity.authority_tier }})</span>
          </div>
        </div>
        <div class="row">
          <button class="btn" type="button" [attr.aria-pressed]="d.opportunity.bookmarked" (click)="toggle('opportunities.bookmark', !d.opportunity.bookmarked)">{{ d.opportunity.bookmarked ? '★ Bookmarked' : '☆ Bookmark' }}</button>
          <button class="btn btn--ghost" type="button" (click)="toggle('opportunities.hide', !d.opportunity.hidden)">{{ d.opportunity.hidden ? 'Unhide' : 'Hide' }}</button>
        </div>
      </header>

      <oc-trust-display [trust]="d.opportunity.trust" [officialUrl]="d.opportunity.official_url" [opportunityId]="d.opportunity.id" (reported)="reload()" />

      <div class="tabs" role="tablist" aria-label="Opportunity sections">
        @for (t of tabs; track t[0]) {
          <button role="tab" type="button" [attr.aria-selected]="tab() === t[0]" [attr.id]="'tab-' + t[0]" [attr.aria-controls]="'panel-' + t[0]" (click)="tab.set(t[0])">{{ t[1] }}</button>
        }
      </div>

      <section role="tabpanel" [attr.id]="'panel-' + tab()" [attr.aria-labelledby]="'tab-' + tab()">
        @switch (tab()) {
          @case ('summary') {
            @if (d.opportunity.summary) { <p>{{ d.opportunity.summary }}</p> }
            <dl class="dl">
              <dt>Benefit</dt><dd>{{ benefit() }}</dd>
              <dt>Claim method</dt><dd>{{ humanize(d.opportunity.claim_method) }}</dd>
              <dt>Proof required</dt><dd>{{ humanize(d.opportunity.proof_required) }}</dd>
              <dt>Class period</dt><dd>{{ dateOnly(d.opportunity.class_period_start) }} – {{ dateOnly(d.opportunity.class_period_end) }}</dd>
              <dt>Jurisdictions</dt><dd>{{ d.opportunity.jurisdictions.join(', ') || '—' }}</dd>
              @if (d.opportunity.administrator_phone) { <dt>Administrator phone</dt><dd>{{ d.opportunity.administrator_phone }}</dd> }
              @if (d.opportunity.court) { <dt>Court</dt><dd>{{ d.opportunity.court }}</dd> }
              <dt>Official URL</dt><dd><a [href]="d.opportunity.official_url" target="_blank" rel="noopener noreferrer">{{ d.opportunity.official_url }}</a></dd>
              @if (d.opportunity.claim_url) { <dt>Claim URL</dt><dd><a [href]="d.opportunity.claim_url" target="_blank" rel="noopener noreferrer">{{ d.opportunity.claim_url }}</a></dd> }
              <dt>Source updated on</dt><dd>{{ dateOnly(d.opportunity.source_updated_on) }}</dd>
              <dt>First seen</dt><dd>{{ date(d.opportunity.first_seen_at) }}</dd>
              <dt>Record version</dt><dd>{{ d.opportunity.version }}</dd>
            </dl>
          }
          @case ('eligibility') {
            <h2>Verified rules</h2>
            @if (d.rules?.rules; as rules) {
              <pre class="card mono small">{{ pretty(rules) }}</pre>
              <p class="hint">Rules version {{ d.rules!.version }}. Matching is a deterministic three-valued evaluation; it never decides eligibility.</p>
            } @else {
              <p class="notice notice--warn">{{ d.rules?.note || 'No verified structured rules yet; matching reports manual review.' }}</p>
            }
            <h2 style="margin-top:1rem">Match explanation</h2>
            <oc-person-select (changed)="loadExplain()" />
            @if (explain(); as ex) {
              <oc-match-explain [explain]="ex" [personId]="people.selectedId()!" (changed)="loadExplain()" />
            } @else if (people.people().length === 0) {
              <p class="muted">Create a person under <a routerLink="/matches">My Matches</a> to evaluate this opportunity.</p>
            }
          }
          @case ('sources') {
            <p class="hint">Evidence excerpts are third-party text captured from the official page. They are shown as plain text and can never change anything in OpenClaim.</p>
            @for (ev of d.provenance?.evidence ?? []; track ev.id) {
              <article class="card" style="margin-bottom: 0.75rem" data-evidence>
                <div class="row row--between">
                  <strong>{{ humanize(ev.assertion_type) }}</strong>
                  <span class="small muted">{{ humanize(ev.acquisition) }} · fetched {{ date(ev.fetched_at) }}</span>
                </div>
                <oc-untrusted-text [text]="ev.excerpt.text" [origin]="ev.excerpt.origin" />
                <p class="small faint" style="margin-top:0.4rem">Locator: <code>{{ ev.precise_locator }}</code> · hash <code>{{ ev.content_hash.slice(0, 12) }}…</code> · parser {{ ev.parser_version }}</p>
              </article>
            } @empty {
              <p class="muted">No provenance evidence stored for this record.</p>
            }
          }
          @case ('evidence') {
            <dl class="dl">
              <dt>Proof required</dt><dd>{{ humanize(d.opportunity.proof_required) }}</dd>
              <dt>Claim method</dt><dd>{{ humanize(d.opportunity.claim_method) }}</dd>
            </dl>
            @if (d.opportunity.proof_required === 'none') {
              <p class="notice notice--ok">The source says no proof is required. Keep any receipts or notices in the Evidence Vault anyway.</p>
            } @else {
              <p>Collect receipts, account statements or notices that show you are in the class. Import them in the <a routerLink="/vault">Evidence Vault</a> and link them to your claim.</p>
            }
            @if (claim(); as c) {
              <p><a class="btn" [routerLink]="['/claims', c.id]">Open claim checklist</a></p>
            }
          }
          @case ('filing') {
            <p class="notice">OpenClaim never submits anything. It opens the official site in a new tab; CAPTCHA, identity checks, signature and the final Submit are yours.</p>
            <oc-person-select />
            @if (claim(); as c) {
              <p>A claim record exists for this person: <a [routerLink]="['/claims', c.id]">{{ humanize(c.status) }}</a>.</p>
            } @else {
              <button class="btn btn--primary" type="button" (click)="startClaim()" [disabled]="!people.selectedId() || busy()">Start tracking a claim</button>
            }
            @if (handoff(); as h) {
              <div class="card" style="margin-top: 1rem">
                <h3>Official filing handoff</h3>
                <p><a class="btn btn--primary" [href]="h.open_url" target="_blank" rel="noopener noreferrer">Open official site ↗</a> <span class="small muted">({{ h.allowed_domain }})</span></p>
                <h4>You must complete:</h4>
                <ol>@for (i of h.instructions; track i) { <li>{{ i }}</li> }</ol>
                <p class="small muted">{{ h.note }}</p>
              </div>
            } @else if (claim()) {
              <button class="btn" type="button" (click)="getHandoff()" [disabled]="busy()">Show official filing handoff</button>
            }
          }
          @case ('tracking') {
            @if (claim(); as c) {
              <p>Status: <oc-badge [value]="c.status" /> · <a [routerLink]="['/claims', c.id]">Open claim</a></p>
            } @else {
              <p class="muted">No claim is tracked for the selected person yet. Start one from the Official filing tab.</p>
            }
            <h3>Change history</h3>
            <ul class="plain">
              @for (c of changes(); track c.field + c.detected_at) {
                <li><strong>{{ c.field }}</strong> <span class="muted small">{{ date(c.detected_at) }}</span><br /><span class="small">{{ c.old_value ?? '∅' }} → {{ c.new_value ?? '∅' }}</span></li>
              } @empty { <li class="muted">No recorded changes.</li> }
            </ul>
          }
        }
      </section>
    } @else if (error()) {
      <p class="notice notice--danger" role="alert">{{ error() }}</p>
    } @else {
      <p class="muted" role="status">Loading…</p>
    }
  `,
})
export class OpportunityDetailPage implements OnInit {
  private readonly api = inject(ApiService);
  private readonly toast = inject(ToastService);
  private readonly router = inject(Router);
  readonly people = inject(PeopleService);

  readonly id = input.required<string>();
  readonly tabs = TABS;
  readonly tab = signal<Tab>('summary');
  readonly detail = signal<OpportunityDetail | null>(null);
  readonly changes = signal<ChangeItem[]>([]);
  readonly explain = signal<MatchExplain | null>(null);
  readonly claim = signal<Claim | null>(null);
  readonly handoff = signal<Handoff | null>(null);
  readonly error = signal<string | null>(null);
  readonly busy = signal(false);

  readonly humanize = humanize;
  readonly date = date;
  readonly dateOnly = dateOnly;
  readonly benefit = computed(() => {
    const b = this.detail()?.opportunity.benefit;
    if (!b) return 'unknown';
    if (b.amount_known && typeof b.amount_minor === 'number') return money(b.amount_minor, b.currency ?? 'USD');
    return b.description ?? 'unknown';
  });

  async ngOnInit(): Promise<void> {
    await this.reload();
    await this.people.load();
    await Promise.all([this.loadExplain(), this.loadClaim()]);
  }

  async reload(): Promise<void> {
    try {
      const d = await this.api.op<OpportunityDetail>('opportunities.show', { id: this.id(), include: ['sources', 'rules', 'provenance', 'changes'] });
      this.detail.set(d);
      const ch = d.changes;
      this.changes.set(Array.isArray(ch) ? ch : ch?.items ?? []);
    } catch (e) {
      this.error.set(e instanceof Error ? e.message : String(e));
    }
  }

  async loadExplain(): Promise<void> {
    const pid = this.people.selectedId();
    if (!pid) { this.explain.set(null); return; }
    try {
      this.explain.set(await this.api.op<MatchExplain>('match.explain', { person_id: pid, opportunity_id: this.id() }));
    } catch (e) {
      this.explain.set(null);
      this.toast.error(e);
    }
    await this.loadClaim();
  }

  async loadClaim(): Promise<void> {
    const pid = this.people.selectedId();
    try {
      const list = await this.api.op<Paged<Claim>>('claims.list', { limit: 200 });
      this.claim.set((list.items ?? []).find((c) => c.opportunity_id === this.id() && (!pid || c.person_id === pid)) ?? null);
    } catch { this.claim.set(null); }
  }

  async toggle(op: string, on: boolean): Promise<void> {
    try {
      await this.api.op(op, { id: this.id(), on });
      await this.reload();
    } catch (e) { this.toast.error(e); }
  }

  async startClaim(): Promise<void> {
    const pid = this.people.selectedId();
    if (!pid) return;
    this.busy.set(true);
    try {
      const r = await this.api.op<{ id: string; existing: boolean }>('claims.init', { person_id: pid, opportunity_id: this.id() });
      this.toast.ok(r.existing ? 'Claim already tracked.' : 'Claim tracking started.');
      await this.router.navigate(['/claims', r.id]);
    } catch (e) { this.toast.error(e); } finally { this.busy.set(false); }
  }

  async getHandoff(): Promise<void> {
    const c = this.claim();
    if (!c) return;
    this.busy.set(true);
    try {
      this.handoff.set(await this.api.op<Handoff>('claims.handoff', { claim_id: c.id }));
      await this.loadClaim();
    } catch (e) { this.toast.error(e); } finally { this.busy.set(false); }
  }

  pretty(v: unknown): string { return JSON.stringify(v, null, 2); }
}
