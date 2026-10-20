import { ChangeDetectionStrategy, Component, OnInit, computed, inject, signal } from '@angular/core';
import { RouterLink } from '@angular/router';
import { ApiService } from '../../core/api.service';
import { Alert, ChangeItem, MatchItem, Paged, SourceHealth } from '../../core/api.types';
import { date, dateOnly, relative } from '../../core/format';
import { ToastService } from '../../core/toast.service';
import { BadgeComponent } from '../../shared/badge.component';
import { PageHeaderComponent } from '../../shared/page-header.component';
import { PeopleService } from '../../shared/people.service';
import { TermComponent } from '../../shared/term.component';

interface PersonMatches { personId: string; name: string; items: MatchItem[] }

@Component({
  selector: 'oc-overview',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [RouterLink, BadgeComponent, PageHeaderComponent, TermComponent],
  template: `
    <oc-page-header title="Overview" subtitle="What OpenClaim found for the people in this profile, with sources and deadlines you can check yourself." />

    <div class="grid grid--3" role="list" aria-label="Summary">
      <div class="card card--tile" role="listitem">
        <span class="tile__value" data-tile="potential">{{ potential().length }}</span>
        <span class="tile__label"><oc-term key="potential_match">Potential matches</oc-term></span>
      </div>
      <div class="card card--tile" role="listitem">
        <span class="tile__value" data-tile="review">{{ needsReview().length }}</span>
        <span class="tile__label">Needs review</span>
      </div>
      <div class="card card--tile" role="listitem">
        <span class="tile__value" data-tile="deadlines">{{ upcoming().length }}</span>
        <span class="tile__label">Upcoming deadlines</span>
      </div>
    </div>

    @if (loading()) { <p class="muted" role="status">Loading…</p> }

    <section class="section" aria-labelledby="matched-h">
      <h2 id="matched-h">Matched opportunities</h2>
      @if (!loading() && allMatches().length === 0) {
        <p class="muted">No evaluated matches yet. Add a person and facts under <a routerLink="/matches">My Matches</a>, or browse <a routerLink="/discover">Discover</a>.</p>
      }
      <div class="stack">
        @for (pm of matches(); track pm.personId) {
          @for (m of pm.items; track m.opportunity_id) {
            <article class="card opp">
              <div class="row row--between">
                <a class="opp__title" [routerLink]="['/discover', m.opportunity_id]">{{ m.title }}</a>
                <oc-badge [value]="m.assessment" />
              </div>
              <div class="opp__meta">
                <span>For {{ pm.name }}</span>
                <span>· <oc-badge [value]="m.verification_state" /></span>
                <span>· Deadline {{ m.claim_deadline ? dateOnly(m.claim_deadline) : 'unknown' }}{{ m.claim_deadline ? (m.deadline_verified ? ' (verified)' : ' (not verified)') : '' }}</span>
                @if (m.stale) { <span>· evaluation is stale</span> }
              </div>
              <div class="row">
                <a class="btn btn--sm" [routerLink]="['/matches']" [queryParams]="{ person: pm.personId, opportunity: m.opportunity_id }">Why it matches</a>
                <a class="btn btn--sm" [href]="m.official_url" target="_blank" rel="noopener noreferrer">Official source ↗</a>
              </div>
            </article>
          }
        }
      </div>
    </section>

    <div class="grid grid--2 section">
      <section class="card" aria-labelledby="deadlines-h">
        <div class="row row--between"><h2 id="deadlines-h">Next deadlines</h2><button class="btn btn--sm" type="button" (click)="exportIcs()" [disabled]="alerts().length === 0">Export .ics</button></div>
        @if (alerts().length === 0) { <p class="muted">No reminders. Reminders exist only for verified deadlines.</p> }
        <ul class="plain">
          @for (a of alerts(); track a.id) {
            <li>
              <div class="row row--between">
                <span>{{ a.title }} @if (a.disposition !== 'pending') { <span class="badge">{{ a.disposition }}</span> }</span>
                <span class="muted small">{{ date(a.due_at) }} · {{ relative(a.due_at) }}</span>
              </div>
              @if (a.opportunity_title) { <div class="small faint">{{ a.opportunity_title }}</div> }
              @if (a.disposition === 'pending') {
                <div class="row" style="margin-top:0.3rem">
                  <button class="btn btn--sm btn--ghost" type="button" (click)="snooze(a)">Snooze 7 days</button>
                  <button class="btn btn--sm btn--ghost" type="button" (click)="dismiss(a)">Dismiss</button>
                </div>
              }
            </li>
          }
        </ul>
      </section>
      <section class="card" aria-labelledby="updates-h">
        <h2 id="updates-h">Recent verified updates</h2>
        @if (changes().length === 0) { <p class="muted">No changes recorded yet.</p> }
        <ul class="plain">
          @for (c of changes(); track c.opportunity_id + c.field + c.detected_at) {
            <li>
              <a [routerLink]="['/discover', c.opportunity_id]">{{ c.title }}</a>
              <span class="muted small"> · {{ c.field }} changed · {{ relative(c.detected_at) }}</span>
            </li>
          }
        </ul>
      </section>
    </div>

    <section class="card section" aria-labelledby="sources-h">
      <h2 id="sources-h">Source health</h2>
      <p class="notice notice--warn small"><oc-term key="coverage_gap">A blocked, stale or disabled source means a coverage gap</oc-term>: results may be incomplete. It never means there are no settlements.</p>
      <div class="table-wrap">
        <table>
          <thead><tr><th scope="col">Source</th><th scope="col">Health</th><th scope="col">Last OK</th><th scope="col">Problem</th></tr></thead>
          <tbody>
            @for (s of sources(); track s.id) {
              <tr>
                <td><a routerLink="/sources" [queryParams]="{ source: s.id }">{{ s.name }}</a></td>
                <td><oc-badge [value]="s.enabled ? s.health : 'disabled'" /></td>
                <td>{{ s.last_ok_at ? relative(s.last_ok_at) : 'never' }}</td>
                <td class="small muted">{{ s.last_failure_detail || s.last_failure || '—' }}</td>
              </tr>
            }
          </tbody>
        </table>
      </div>
    </section>
  `,
})
export class OverviewPage implements OnInit {
  private readonly api = inject(ApiService);
  private readonly toast = inject(ToastService);
  private readonly people = inject(PeopleService);

  readonly loading = signal(true);
  readonly matches = signal<PersonMatches[]>([]);
  readonly alerts = signal<Alert[]>([]);
  readonly changes = signal<ChangeItem[]>([]);
  readonly sources = signal<SourceHealth[]>([]);

  readonly allMatches = computed(() => this.matches().flatMap((m) => m.items));
  readonly potential = computed(() => this.allMatches().filter((m) => m.assessment === 'potential_match'));
  readonly needsReview = computed(() => this.allMatches().filter((m) => m.assessment === 'needs_more_information' || m.assessment === 'manual_review'));
  readonly upcoming = computed(() => this.alerts().filter((a) => a.disposition === 'pending' && new Date(a.due_at).getTime() >= Date.now()));

  readonly date = date;
  readonly dateOnly = dateOnly;
  readonly relative = relative;

  async snooze(a: Alert): Promise<void> {
    const until = new Date(Date.now() + 7 * 86400000).toISOString();
    try { await this.api.op('alerts.snooze', { id: a.id, until }); this.toast.ok('Snoozed for 7 days.'); await this.ngOnInit(); } catch (e) { this.toast.error(e); }
  }

  async dismiss(a: Alert): Promise<void> {
    try { await this.api.op('alerts.dismiss', { id: a.id }); this.toast.ok('Dismissed.'); await this.ngOnInit(); } catch (e) { this.toast.error(e); }
  }

  /** alerts.export_ics without a path returns the calendar text; offer it as a download. */
  async exportIcs(): Promise<void> {
    try {
      const r = await this.api.op<unknown>('alerts.export_ics', {});
      const text = typeof r === 'string' ? r : (r as { ics?: string; content?: string })?.ics ?? (r as { content?: string })?.content ?? JSON.stringify(r);
      const blob = new Blob([text], { type: 'text/calendar' });
      const a = document.createElement('a'); a.href = URL.createObjectURL(blob); a.download = 'openclaim-reminders.ics'; a.click(); URL.revokeObjectURL(a.href);
    } catch (e) { this.toast.error(e); }
  }

  async ngOnInit(): Promise<void> {
    try {
      const people = await this.people.load();
      const [matches, alerts, changes, health] = await Promise.all([
        Promise.all(people.map(async (p) => ({ personId: p.id, name: p.display_name, items: (await this.api.op<Paged<MatchItem>>('match.list', { person_id: p.id })).items ?? [] }))),
        this.api.op<Paged<Alert>>('alerts.list', {}),
        this.api.op<Paged<ChangeItem>>('discover.history', { limit: 8 }),
        this.api.op<Paged<SourceHealth>>('sources.health', {}),
      ]);
      this.matches.set(matches);
      this.alerts.set((alerts.items ?? []).sort((a, b) => a.due_at.localeCompare(b.due_at)));
      this.changes.set(changes.items ?? []);
      this.sources.set(health.items ?? []);
    } catch (e) {
      this.toast.error(e);
    } finally {
      this.loading.set(false);
    }
  }
}
