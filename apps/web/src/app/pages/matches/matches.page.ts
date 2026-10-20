import { ChangeDetectionStrategy, Component, OnInit, computed, inject, signal } from '@angular/core';
import { ActivatedRoute, RouterLink } from '@angular/router';
import { ApiService } from '../../core/api.service';
import { MatchExplain, MatchItem, MatchState, Paged, Person } from '../../core/api.types';
import { date, dateOnly, humanize } from '../../core/format';
import { ToastService } from '../../core/toast.service';
import { BadgeComponent } from '../../shared/badge.component';
import { PageHeaderComponent } from '../../shared/page-header.component';
import { PeopleService } from '../../shared/people.service';
import { PersonSelectComponent } from '../../shared/person-select.component';
import { TermComponent } from '../../shared/term.component';
import { MatchExplainComponent } from './match-explain.component';

const ORDER: MatchState[] = ['potential_match', 'needs_more_information', 'manual_review', 'unlikely', 'not_applicable'];

@Component({
  selector: 'oc-matches',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [RouterLink, BadgeComponent, PageHeaderComponent, PersonSelectComponent, MatchExplainComponent, TermComponent],
  template: `
    <oc-page-header title="My Matches" subtitle="Deterministic rule evaluation of your locally stored facts. Only the official administrator decides eligibility.">
      <oc-person-select (changed)="load()" />
      <button class="btn" type="button" (click)="evaluateAll()" [disabled]="!people.selectedId() || busy()">Re-evaluate all</button>
      <button class="btn btn--ghost" type="button" (click)="creating.set(!creating())">New person</button>
    </oc-page-header>

    @if (creating()) {
      <form class="card" (submit)="createPerson($event)" style="margin-bottom:1rem">
        <h2>New person</h2>
        <div class="field"><label for="np-name">Display name</label><input id="np-name" type="text" [value]="newName()" (input)="newName.set($any($event.target).value)" required /></div>
        <label class="field field--inline"><input type="checkbox" [checked]="newConsent()" (change)="newConsent.set($any($event.target).checked)" /> I am an adult and authorize storing this person's facts on this computer.</label>
        <div class="row"><button class="btn btn--primary" type="submit" [disabled]="!newName().trim() || !newConsent() || busy()">Create</button><button class="btn btn--ghost" type="button" (click)="creating.set(false)">Cancel</button></div>
      </form>
    }

    @if (person(); as p) {
      <details class="card" style="margin-bottom:1rem">
        <summary>Profile: {{ p.display_name }} · {{ p.residences?.length ?? 0 }} residence period(s) · {{ p.consumer_fact_count ?? 0 }} fact(s)</summary>
        <form class="row" (submit)="addResidence($event)" style="margin-top:0.75rem">
          <div class="field" style="margin:0"><label for="res-j">Add residence (jurisdiction)</label><input id="res-j" type="text" placeholder="US-CA" [value]="resJur()" (input)="resJur.set($any($event.target).value)" style="width: 8rem" /></div>
          <div class="field" style="margin:0"><label for="res-s">From</label><input id="res-s" type="date" [value]="resStart()" (input)="resStart.set($any($event.target).value)" /></div>
          <div class="field" style="margin:0"><label for="res-e">To</label><input id="res-e" type="date" [value]="resEnd()" (input)="resEnd.set($any($event.target).value)" /></div>
          <button class="btn btn--sm" type="submit" [disabled]="!resJur().trim() || busy()" style="align-self:flex-end">Add</button>
        </form>
        <ul class="plain">
          @for (r of p.residences ?? []; track $index) {
            <li class="row row--between"><span>{{ r.jurisdiction }} {{ r.start || '?' }} – {{ r.end || 'present' }}</span>
              @if (r.id) { <button class="btn btn--sm btn--ghost" type="button" (click)="removeResidence(r.id!)">Remove</button> }</li>
          }
        </ul>
      </details>
    }

    @if (loading()) { <p class="muted" role="status">Loading…</p> }
    @if (!loading() && items().length === 0 && people.selectedId()) {
      <p class="card muted">No evaluations yet for this person. Click "Re-evaluate all".</p>
    }

    @for (g of groups(); track g.state) {
      <section class="section" [attr.aria-labelledby]="'g-' + g.state">
        <h2 [id]="'g-' + g.state"><oc-badge [value]="g.state" /> <span class="muted small">{{ g.items.length }}</span></h2>
        <div class="stack">
          @for (m of g.items; track m.opportunity_id) {
            <article class="card opp" [attr.data-opportunity-id]="m.opportunity_id">
              <div class="row row--between">
                <a class="opp__title" [routerLink]="['/discover', m.opportunity_id]">{{ m.title }}</a>
                <div class="row">
                  <oc-badge [value]="m.verification_state" />
                  <button class="btn btn--sm" type="button" [attr.aria-expanded]="selected() === m.opportunity_id" (click)="toggleExplain(m.opportunity_id)">{{ selected() === m.opportunity_id ? 'Hide reasons' : 'Why?' }}</button>
                  <a class="btn btn--sm" [href]="m.official_url" target="_blank" rel="noopener noreferrer">Official source ↗</a>
                </div>
              </div>
              <div class="opp__meta">
                <span>{{ humanize(m.status) }}</span>
                <span>· <oc-term key="claim_deadline">Deadline</oc-term> {{ m.claim_deadline ? dateOnly(m.claim_deadline) : 'unknown' }}{{ m.claim_deadline ? (m.deadline_verified ? ' (verified)' : ' (not verified)') : '' }}</span>
                <span>· evaluated {{ date(m.evaluated_at) }}</span>
                @if (m.stale) { <span>· stale</span> }
              </div>
              @if (selected() === m.opportunity_id && explain(); as ex) {
                <oc-match-explain [explain]="ex" [personId]="people.selectedId()!" (changed)="refreshExplain()" />
              }
            </article>
          }
        </div>
      </section>
    }

    @if (proposals().length) {
      <section class="section card" aria-labelledby="prop-h">
        <h2 id="prop-h">Agent-proposed facts ({{ proposals().length }})</h2>
        <p class="hint">Proposed by an external agent or extraction. Not used for matching until you confirm.</p>
        @for (p of proposals(); track $index) {
          <div class="agent-proposed" style="margin-bottom:0.5rem">
            <span class="agent-proposed__label">Agent proposed · unconfirmed</span>
            <div class="row row--between">
              <span>{{ proposalText(p) }}</span>
              @if (proposalId(p); as pid) {
                <span class="row">
                  <button class="btn btn--sm" type="button" (click)="decideProposal(pid, true)" [disabled]="busy()">Confirm</button>
                  <button class="btn btn--sm btn--ghost" type="button" (click)="decideProposal(pid, false)" [disabled]="busy()">Reject</button>
                </span>
              }
            </div>
          </div>
        }
      </section>
    }

    @if (missing().length) {
      <section class="section card" aria-labelledby="missing-h">
        <h2 id="missing-h">Open questions</h2>
        <p class="hint">Answering these lets rules evaluate. Only answer what is true.</p>
        <ul class="plain">
          @for (q of missing(); track $index) {
            <li class="row row--between">
              <span>{{ missingText(q) }}</span>
              @if (missingOpp(q); as oid) {
                <span class="row">
                  <button class="btn btn--sm" type="button" (click)="attest(oid, missingKey(q), true)" [disabled]="busy()">Yes</button>
                  <button class="btn btn--sm" type="button" (click)="attest(oid, missingKey(q), false)" [disabled]="busy()">No</button>
                </span>
              }
            </li>
          }
        </ul>
      </section>
    }
  `,
})
export class MatchesPage implements OnInit {
  private readonly api = inject(ApiService);
  private readonly toast = inject(ToastService);
  private readonly route = inject(ActivatedRoute);
  readonly people = inject(PeopleService);

  readonly loading = signal(false);
  readonly busy = signal(false);
  readonly items = signal<MatchItem[]>([]);
  readonly missing = signal<unknown[]>([]);
  readonly proposals = signal<unknown[]>([]);
  readonly selected = signal<string | null>(null);
  readonly explain = signal<MatchExplain | null>(null);
  readonly person = signal<Person | null>(null);

  readonly creating = signal(false);
  readonly newName = signal('');
  readonly newConsent = signal(false);
  readonly resJur = signal('');
  readonly resStart = signal('');
  readonly resEnd = signal('');

  readonly groups = computed(() => ORDER.map((state) => ({ state, items: this.items().filter((m) => m.assessment === state) })).filter((g) => g.items.length > 0));

  readonly humanize = humanize;
  readonly date = date;
  readonly dateOnly = dateOnly;

  async ngOnInit(): Promise<void> {
    await this.people.load();
    const qp = this.route.snapshot.queryParamMap;
    if (qp.get('person')) this.people.select(qp.get('person'));
    await this.load();
    if (qp.get('opportunity')) await this.toggleExplain(qp.get('opportunity')!);
  }

  async load(): Promise<void> {
    const pid = this.people.selectedId();
    this.selected.set(null);
    this.explain.set(null);
    if (!pid) { this.items.set([]); this.person.set(null); return; }
    this.loading.set(true);
    try {
      const [list, missing, person, history] = await Promise.all([
        this.api.op<Paged<MatchItem>>('match.list', { person_id: pid }),
        this.api.op<Paged<unknown>>('match.missing', { person_id: pid }),
        this.api.op<Person>('people.show', { id: pid }),
        this.api.op<{ proposals?: unknown[] }>('facts.history', { person_id: pid }).catch(() => ({ proposals: [] })),
      ]);
      this.items.set(list.items ?? []);
      this.missing.set(missing.items ?? []);
      this.person.set(person);
      this.proposals.set((history.proposals ?? []).filter((p) => { const o = p as Record<string, unknown>; return !o['status'] || o['status'] === 'proposed' || o['status'] === 'pending'; }));
    } catch (e) { this.toast.error(e); } finally { this.loading.set(false); }
  }

  async evaluateAll(): Promise<void> {
    const pid = this.people.selectedId();
    if (!pid) return;
    this.busy.set(true);
    try {
      const r = await this.api.op<{ evaluated: number }>('match.evaluate', { person_id: pid });
      this.toast.ok(`Evaluated ${r.evaluated} opportunit${r.evaluated === 1 ? 'y' : 'ies'}.`);
      await this.load();
    } catch (e) { this.toast.error(e); } finally { this.busy.set(false); }
  }

  async toggleExplain(oid: string): Promise<void> {
    if (this.selected() === oid) { this.selected.set(null); this.explain.set(null); return; }
    this.selected.set(oid);
    await this.refreshExplain();
  }

  async refreshExplain(): Promise<void> {
    const pid = this.people.selectedId();
    const oid = this.selected();
    if (!pid || !oid) return;
    try {
      this.explain.set(await this.api.op<MatchExplain>('match.explain', { person_id: pid, opportunity_id: oid }));
      const list = await this.api.op<Paged<MatchItem>>('match.list', { person_id: pid });
      this.items.set(list.items ?? []);
    } catch (e) { this.toast.error(e); }
  }

  proposalText(p: unknown): string {
    const o = p as Record<string, unknown>;
    const pr = (o['proposal'] as Record<string, unknown> | undefined) ?? o;
    const bits = [pr['fact_type'], pr['merchant'], pr['product'], pr['start'] ? `from ${String(pr['start'])}` : null, pr['end'] ? `to ${String(pr['end'])}` : null, o['agent_name'] ? `(by ${String(o['agent_name'])})` : null];
    return bits.filter(Boolean).map(String).join(' ') || JSON.stringify(p);
  }
  proposalId(p: unknown): string | null { const o = p as Record<string, unknown>; return typeof o['id'] === 'string' ? o['id'] : null; }
  async decideProposal(id: string, confirm: boolean): Promise<void> {
    this.busy.set(true);
    try { await this.api.op(confirm ? 'facts.confirm' : 'facts.reject', { id }); this.toast.ok(confirm ? 'Fact confirmed.' : 'Proposal rejected.'); await this.load(); } catch (e) { this.toast.error(e); } finally { this.busy.set(false); }
  }

  missingText(q: unknown): string {
    if (typeof q === 'string') return q;
    const o = q as Record<string, unknown>;
    const base = String(o['question'] ?? o['label'] ?? o['key'] ?? JSON.stringify(q)).replace(/_/g, ' ');
    return o['title'] ? `${base} (${String(o['title'])})` : base;
  }
  missingKey(q: unknown): string { const o = q as Record<string, unknown>; return typeof q === 'string' ? q : String(o['key'] ?? o['fact'] ?? ''); }
  missingOpp(q: unknown): string | null { const o = q as Record<string, unknown>; return typeof q === 'string' ? null : ((o['opportunity_id'] as string | undefined) ?? null); }

  async attest(oid: string, key: string, value: boolean): Promise<void> {
    const pid = this.people.selectedId();
    if (!pid) return;
    this.busy.set(true);
    try {
      await this.api.op('facts.attest', { person_id: pid, opportunity_id: oid, key, value });
      this.toast.ok('Answer recorded.');
      await this.load();
    } catch (e) { this.toast.error(e); } finally { this.busy.set(false); }
  }

  async createPerson(ev: Event): Promise<void> {
    ev.preventDefault();
    this.busy.set(true);
    try {
      const r = await this.api.op<{ id: string }>('people.create', { display_name: this.newName().trim(), consent: { authorized_by_adult: this.newConsent() } });
      await this.people.load(true);
      this.people.select(r.id);
      this.creating.set(false);
      this.newName.set('');
      this.newConsent.set(false);
      await this.load();
    } catch (e) { this.toast.error(e); } finally { this.busy.set(false); }
  }

  async addResidence(ev: Event): Promise<void> {
    ev.preventDefault();
    const pid = this.people.selectedId();
    if (!pid) return;
    this.busy.set(true);
    try {
      await this.api.op('people.add_residence', { person_id: pid, jurisdiction: this.resJur().trim().toUpperCase(), start: this.resStart() || null, end: this.resEnd() || null });
      this.resJur.set(''); this.resStart.set(''); this.resEnd.set('');
      await this.load();
    } catch (e) { this.toast.error(e); } finally { this.busy.set(false); }
  }

  async removeResidence(id: string): Promise<void> {
    try { await this.api.op('people.remove_residence', { id }); await this.load(); } catch (e) { this.toast.error(e); }
  }
}
