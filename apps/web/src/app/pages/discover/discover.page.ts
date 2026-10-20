import { ChangeDetectionStrategy, Component, OnInit, computed, inject, signal } from '@angular/core';
import { ApiService } from '../../core/api.service';
import { Opportunity, Paged, Pagination } from '../../core/api.types';
import { dateOnly, humanize } from '../../core/format';
import { ToastService } from '../../core/toast.service';
import { BadgeComponent } from '../../shared/badge.component';
import { OpportunityCardComponent } from '../../shared/opportunity-card.component';
import { PageHeaderComponent } from '../../shared/page-header.component';

const STATUSES = ['open', 'automatic_distribution', 'closed', 'pending', 'unknown'];
const SORTS: Array<[string, string]> = [['relevance', 'Relevance'], ['deadline', 'Deadline soonest'], ['verified_at', 'Recently verified'], ['updated_at', 'Recently updated'], ['title', 'Title']];

interface CompareRow { id: string; title: string; [k: string]: unknown }

@Component({
  selector: 'oc-discover',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [BadgeComponent, OpportunityCardComponent, PageHeaderComponent],
  template: `
    <oc-page-header title="Discover" subtitle="Search the local catalog of settlements, refunds and distributions. Every record links to its official source." />

    <form class="card" (submit)="search($event)" aria-label="Search and filters">
      <div class="row">
        <div class="field" style="flex: 2; min-width: 14rem; margin: 0">
          <label for="q">Search</label>
          <input id="q" type="search" [value]="q()" (input)="q.set($any($event.target).value)" placeholder="Company, product, case number…" />
        </div>
        <div class="field" style="margin:0">
          <label for="status">Status</label>
          <select id="status" [value]="status()" (change)="status.set($any($event.target).value)">
            <option value="">Any</option>
            @for (s of statuses; track s) { <option [value]="s">{{ humanize(s) }}</option> }
          </select>
        </div>
        <div class="field" style="margin:0">
          <label for="jur">Jurisdiction</label>
          <input id="jur" type="text" [value]="jurisdiction()" (input)="jurisdiction.set($any($event.target).value)" placeholder="US, US-CA…" style="width: 7rem" />
        </div>
        <div class="field" style="margin:0">
          <label for="deadline">Deadline before</label>
          <input id="deadline" type="date" [value]="deadlineBefore()" (input)="deadlineBefore.set($any($event.target).value)" />
        </div>
        <div class="field" style="margin:0">
          <label for="sort">Sort</label>
          <select id="sort" [value]="sort()" (change)="sort.set($any($event.target).value)">
            @for (s of sorts; track s[0]) { <option [value]="s[0]">{{ s[1] }}</option> }
          </select>
        </div>
      </div>
      <div class="row" style="margin-top: 0.75rem">
        <label class="field field--inline" style="margin:0"><input type="checkbox" [checked]="onlyVerified()" (change)="onlyVerified.set($any($event.target).checked)" /> Only verified</label>
        <label class="field field--inline" style="margin:0"><input type="checkbox" [checked]="bookmarkedOnly()" (change)="bookmarkedOnly.set($any($event.target).checked)" /> Bookmarked only</label>
        <label class="field field--inline" style="margin:0"><input type="checkbox" [checked]="includeHidden()" (change)="includeHidden.set($any($event.target).checked)" /> Include hidden</label>
        <span class="spacer"></span>
        <button class="btn btn--primary" type="submit" [disabled]="loading()">Search</button>
      </div>
    </form>

    <div class="row row--between" style="margin: 1rem 0 0.5rem">
      <p class="muted small" role="status" aria-live="polite">
        @if (loading()) { Searching… } @else { {{ total() }} result{{ total() === 1 ? '' : 's' }}{{ pageInfo() }} }
      </p>
      <div class="row">
        @if (compareIds().length > 0) {
          <span class="small muted">{{ compareIds().length }} selected</span>
          <button class="btn btn--sm" type="button" (click)="compare()" [disabled]="compareIds().length < 2">Compare selected</button>
          <button class="btn btn--sm btn--ghost" type="button" (click)="compareIds.set([]); comparison.set(null)">Clear</button>
        }
      </div>
    </div>

    @if (comparison(); as cmp) {
      <section class="card" aria-labelledby="cmp-h" style="margin-bottom: 1rem">
        <div class="row row--between"><h2 id="cmp-h">Comparison</h2><button class="btn btn--sm btn--ghost" type="button" (click)="comparison.set(null)">Close</button></div>
        <div class="table-wrap">
          <table>
            <thead><tr><th scope="col">Field</th>@for (r of cmp.rows; track r.id) { <th scope="col">{{ r.title }}</th> }</tr></thead>
            <tbody>
              @for (f of cmp.fields; track f) {
                <tr><th scope="row">{{ humanize(f) }}</th>@for (r of cmp.rows; track r.id) { <td>{{ cell(r[f]) }}</td> }</tr>
              }
            </tbody>
          </table>
        </div>
      </section>
    }

    <div class="stack" data-list="opportunities">
      @for (o of items(); track o.id) {
        <oc-opportunity-card [opp]="o" [compareEnabled]="true" [compared]="compareIds().includes(o.id)" (compareToggle)="toggleCompare(o.id, $event)" (changed)="replace($event)" />
      } @empty {
        @if (!loading()) {
          <div class="card">
            <p>No opportunities match. @if (onlyVerified()) { Unverified records are hidden because "Only verified" is on. }</p>
            <p class="muted small">If a source is blocked or stale, this list may be incomplete. Check <strong>Sources</strong> or import a page snapshot you saved from your own browser.</p>
          </div>
        }
      }
    </div>

    <div class="row" style="margin-top: 1rem">
      @if (prevCursors().length > 0) { <button class="btn" type="button" (click)="prev()" [disabled]="loading()">← Previous</button> }
      @if (nextCursor()) { <button class="btn" type="button" (click)="next()" [disabled]="loading()">Next →</button> }
    </div>
    <p class="hint">Records marked <oc-badge value="verification_pending" /> are excluded from verified-only views. Deadlines are only reliable when marked verified.</p>
  `,
})
export class DiscoverPage implements OnInit {
  private readonly api = inject(ApiService);
  private readonly toast = inject(ToastService);

  readonly statuses = STATUSES;
  readonly sorts = SORTS;
  readonly humanize = humanize;

  readonly q = signal('');
  readonly status = signal('');
  readonly jurisdiction = signal('');
  readonly deadlineBefore = signal('');
  readonly sort = signal('relevance');
  readonly onlyVerified = signal(true);
  readonly bookmarkedOnly = signal(false);
  readonly includeHidden = signal(false);

  readonly loading = signal(false);
  readonly items = signal<Opportunity[]>([]);
  readonly pagination = signal<Pagination | null>(null);
  readonly cursor = signal<string | null>(null);
  readonly prevCursors = signal<Array<string | null>>([]);
  readonly nextCursor = computed(() => this.pagination()?.next_cursor ?? null);
  readonly total = computed(() => this.pagination()?.total ?? this.items().length);
  readonly pageInfo = computed(() => (this.prevCursors().length ? ` · page ${this.prevCursors().length + 1}` : ''));

  readonly compareIds = signal<string[]>([]);
  readonly comparison = signal<{ rows: CompareRow[]; fields: string[] } | null>(null);

  readonly dateOnly = dateOnly;

  ngOnInit(): void { void this.load(); }

  async search(ev?: Event): Promise<void> {
    ev?.preventDefault();
    this.cursor.set(null);
    this.prevCursors.set([]);
    await this.load();
  }

  async load(): Promise<void> {
    this.loading.set(true);
    try {
      const params: Record<string, unknown> = {
        q: this.q() || null,
        status: this.status() ? [this.status()] : [],
        only_verified: this.onlyVerified(),
        deadline_before: this.deadlineBefore() || null,
        sort: this.sort() === 'relevance' ? null : this.sort(),
        bookmarked: this.bookmarkedOnly() ? true : null,
        include_hidden: this.includeHidden(),
        limit: 20,
        cursor: this.cursor(),
      };
      if (this.jurisdiction().trim()) params['filter'] = `jurisdiction:${this.jurisdiction().trim()}`;
      const r = await this.api.opFull<Paged<Opportunity>>('opportunities.list', params);
      let items = r.data.items ?? [];
      const j = this.jurisdiction().trim().toUpperCase();
      if (j) items = items.filter((o) => o.jurisdictions.some((x) => x.toUpperCase().startsWith(j)));
      this.items.set(items);
      this.pagination.set(r.envelope.pagination ?? null);
    } catch (e) {
      this.toast.error(e);
    } finally {
      this.loading.set(false);
    }
  }

  async next(): Promise<void> {
    const nc = this.nextCursor();
    if (!nc) return;
    this.prevCursors.update((l) => [...l, this.cursor()]);
    this.cursor.set(nc);
    await this.load();
  }

  async prev(): Promise<void> {
    const list = this.prevCursors();
    if (!list.length) return;
    this.cursor.set(list[list.length - 1]);
    this.prevCursors.set(list.slice(0, -1));
    await this.load();
  }

  replace(o: Opportunity): void {
    this.items.update((l) => l.map((x) => (x.id === o.id ? o : x)));
  }

  toggleCompare(id: string, on: boolean): void {
    this.compareIds.update((l) => (on ? [...new Set([...l, id])].slice(0, 4) : l.filter((x) => x !== id)));
  }

  async compare(): Promise<void> {
    try {
      const data = await this.api.op<Record<string, unknown>>('discover.compare', { ids: this.compareIds() });
      const raw = (data['items'] ?? data['opportunities'] ?? data['rows'] ?? []) as CompareRow[];
      const fields = (data['fields'] as string[] | undefined) ?? ['status', 'verification_state', 'claim_deadline', 'deadline_verified', 'claim_method', 'proof_required', 'administrator', 'jurisdictions', 'benefit', 'official_url'];
      this.comparison.set({ rows: raw, fields: fields.filter((f) => f !== 'id' && f !== 'title') });
    } catch (e) {
      this.toast.error(e);
    }
  }

  cell(v: unknown): string {
    if (v === null || v === undefined) return '—';
    if (typeof v === 'boolean') return v ? 'yes' : 'no';
    if (Array.isArray(v)) return v.map((x) => this.cell(x)).join(', ');
    if (typeof v === 'object') {
      const b = v as { description?: string; amount_minor?: number };
      return b.description ?? JSON.stringify(v);
    }
    return String(v);
  }
}
