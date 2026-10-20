import { ChangeDetectionStrategy, Component, OnInit, computed, inject, signal } from '@angular/core';
import { ApiService } from '../../core/api.service';
import { Paged } from '../../core/api.types';
import { date, dateOnly, humanize } from '../../core/format';
import { ToastService } from '../../core/toast.service';
import { BadgeComponent } from '../../shared/badge.component';
import { PageHeaderComponent } from '../../shared/page-header.component';
import { PeopleService } from '../../shared/people.service';
import { PersonSelectComponent } from '../../shared/person-select.component';

interface StateRow { jurisdiction: string; name: string; official_url: string; kind: string; automated_search: boolean; instructions: string; verified_at: string | null }
interface Checklist { person_id: string; steps: Array<{ jurisdiction?: string; name?: string; official_url?: string; status?: string; instructions?: string; [k: string]: unknown }>; also_check: Array<{ jurisdiction: string; name: string; official_url: string; instructions: string }>; note?: string }
interface Heir { title: string; steps: string[]; note: string }
interface HistoryRow { jurisdiction?: string; status?: string; note?: string | null; heir_claim?: boolean | null; recorded_at?: string; timestamp?: string; [k: string]: unknown }

const STATUSES = ['not_started', 'searched', 'possible_match', 'claim_opened', 'submitted', 'paid', 'no_match'];

@Component({
  selector: 'oc-property',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [BadgeComponent, PageHeaderComponent, PersonSelectComponent],
  template: `
    <oc-page-header title="Property Search" subtitle="Official unclaimed-property programs. Searches happen on the official sites; OpenClaim only tracks what you did.">
      <oc-person-select (changed)="loadPerson()" />
    </oc-page-header>

    <section class="card" aria-labelledby="chk-h">
      <h2 id="chk-h">Checklist for {{ people.selected()?.display_name || 'selected person' }}</h2>
      @if (checklist(); as c) {
        @if (c.note) { <p class="hint">{{ c.note }}</p> }
        @if (c.steps.length) {
          <ol>@for (s of c.steps; track $index) {
            <li>{{ s.name || s.jurisdiction }} @if (s.status) { <oc-badge [value]="s.status" /> } @if (s.official_url) { · <a [href]="s.official_url" target="_blank" rel="noopener noreferrer">official site ↗</a> } @if (s.instructions) { <div class="small muted">{{ s.instructions }}</div> }</li>
          }</ol>
        }
        <h3>Also check</h3>
        <ul>@for (a of c.also_check; track a.jurisdiction) { <li><a [href]="a.official_url" target="_blank" rel="noopener noreferrer">{{ a.name }} ↗</a> <span class="small muted">— {{ a.instructions }}</span></li> }</ul>
      } @else { <p class="muted">Select a person to build a checklist.</p> }
    </section>

    <section class="card section" aria-labelledby="track-h">
      <h2 id="track-h">Track a search</h2>
      <form class="row" (submit)="track($event)">
        <div class="field" style="margin:0"><label for="t-j">Jurisdiction</label>
          <select id="t-j" [value]="tJur()" (change)="tJur.set($any($event.target).value)" style="min-width: 12rem"><option value="">Choose…</option>@for (s of states(); track s.jurisdiction) { <option [value]="s.jurisdiction">{{ s.jurisdiction }} — {{ s.name }}</option> }</select></div>
        <div class="field" style="margin:0"><label for="t-s">Status</label>
          <select id="t-s" [value]="tStatus()" (change)="tStatus.set($any($event.target).value)">@for (s of statuses; track s) { <option [value]="s">{{ humanize(s) }}</option> }</select></div>
        <div class="field" style="margin:0; flex:1"><label for="t-n">Note</label><input id="t-n" type="text" [value]="tNote()" (input)="tNote.set($any($event.target).value)" /></div>
        <label class="field field--inline" style="margin:0"><input type="checkbox" [checked]="tHeir()" (change)="tHeir.set($any($event.target).checked)" /> Heir claim</label>
        <button class="btn btn--primary" type="submit" [disabled]="!tJur() || !people.selectedId() || busy()" style="align-self:flex-end">Record</button>
      </form>
      <h3 style="margin-top:1rem">History</h3>
      <ul class="plain">
        @for (h of history(); track $index) {
          <li><strong>{{ h.jurisdiction }}</strong> <oc-badge [value]="h.status ?? 'unknown'" /> @if (h.heir_claim) { <span class="badge">heir</span> } <span class="small muted">{{ date(h.recorded_at || h.timestamp) }}</span>@if (h.note) { <div class="small">{{ h.note }}</div> }</li>
        } @empty { <li class="muted">Nothing tracked yet.</li> }
      </ul>
    </section>

    <section class="card section" aria-labelledby="heir-h">
      <h2 id="heir-h">{{ heir()?.title || 'Heir checklist' }}</h2>
      @if (heir(); as h) { <ol>@for (s of h.steps; track s) { <li>{{ s }}</li> }</ol><p class="hint">{{ h.note }}</p> }
    </section>

    <section class="card section" aria-labelledby="dir-h">
      <div class="row row--between"><h2 id="dir-h">Official directory</h2>
        <div class="field" style="margin:0"><label for="dir-q" class="visually-hidden">Filter directory</label><input id="dir-q" type="search" placeholder="Filter…" [value]="filter()" (input)="filter.set($any($event.target).value)" /></div></div>
      <div class="table-wrap">
        <table>
          <thead><tr><th scope="col">Code</th><th scope="col">Program</th><th scope="col">Kind</th><th scope="col">Automated search</th><th scope="col">Verified</th><th scope="col">Official link</th></tr></thead>
          <tbody>
            @for (s of filtered(); track s.jurisdiction) {
              <tr><td class="mono">{{ s.jurisdiction }}</td><td>{{ s.name }}<div class="small muted">{{ s.instructions }}</div></td><td>{{ s.kind }}</td><td>{{ s.automated_search ? 'yes' : 'no (manual)' }}</td><td class="small">{{ dateOnly(s.verified_at) }}</td><td><a [href]="s.official_url" target="_blank" rel="noopener noreferrer">Open ↗</a></td></tr>
            }
          </tbody>
        </table>
      </div>
    </section>
  `,
})
export class PropertyPage implements OnInit {
  private readonly api = inject(ApiService);
  private readonly toast = inject(ToastService);
  readonly people = inject(PeopleService);

  readonly states = signal<StateRow[]>([]);
  readonly checklist = signal<Checklist | null>(null);
  readonly heir = signal<Heir | null>(null);
  readonly history = signal<HistoryRow[]>([]);
  readonly filter = signal('');
  readonly busy = signal(false);
  readonly statuses = STATUSES;
  readonly tJur = signal('');
  readonly tStatus = signal('searched');
  readonly tNote = signal('');
  readonly tHeir = signal(false);

  readonly filtered = computed(() => {
    const f = this.filter().trim().toLowerCase();
    return f ? this.states().filter((s) => `${s.jurisdiction} ${s.name} ${s.kind}`.toLowerCase().includes(f)) : this.states();
  });

  readonly humanize = humanize;
  readonly date = date;
  readonly dateOnly = dateOnly;

  async ngOnInit(): Promise<void> {
    try {
      const [states, heir] = await Promise.all([this.api.op<Paged<StateRow>>('property.states', {}), this.api.op<Heir>('property.heir_checklist', {})]);
      this.states.set(states.items ?? []);
      this.heir.set(heir);
      await this.people.load();
      await this.loadPerson();
    } catch (e) { this.toast.error(e); }
  }

  async loadPerson(): Promise<void> {
    const pid = this.people.selectedId();
    if (!pid) { this.checklist.set(null); this.history.set([]); return; }
    try {
      const [c, h] = await Promise.all([this.api.op<Checklist>('property.checklist', { person_id: pid }), this.api.op<Paged<HistoryRow>>('property.history', { person_id: pid })]);
      this.checklist.set(c);
      this.history.set(h.items ?? []);
    } catch (e) { this.toast.error(e); }
  }

  async track(ev: Event): Promise<void> {
    ev.preventDefault();
    const pid = this.people.selectedId();
    if (!pid) return;
    this.busy.set(true);
    try {
      await this.api.op('property.track', { person_id: pid, jurisdiction: this.tJur(), status: this.tStatus(), note: this.tNote() || null, heir_claim: this.tHeir() });
      this.toast.ok('Recorded.');
      this.tNote.set('');
      await this.loadPerson();
    } catch (e) { this.toast.error(e); } finally { this.busy.set(false); }
  }
}
