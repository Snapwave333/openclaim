import { ChangeDetectionStrategy, Component, OnInit, inject, signal } from '@angular/core';
import { ActivatedRoute } from '@angular/router';
import { ApiService } from '../../core/api.service';
import { Paged, Source } from '../../core/api.types';
import { date, humanize, relative } from '../../core/format';
import { ToastService } from '../../core/toast.service';
import { BadgeComponent } from '../../shared/badge.component';
import { ConfirmService } from '../../shared/confirm.service';
import { PageHeaderComponent } from '../../shared/page-header.component';
import { TermComponent } from '../../shared/term.component';

interface HistoryRow { id: string; url: string; content_hash: string; fetched_at: string; acquisition: string; http_status: number | null; change_summary: string | null; parser_version: string }

@Component({
  selector: 'oc-sources',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [BadgeComponent, PageHeaderComponent, TermComponent],
  template: `
    <oc-page-header title="Sources" subtitle="Official registries OpenClaim reads. Every connector is free, respects robots rules and rate limits, and is never bypassed when blocked.">
      <button class="btn" type="button" (click)="refreshAll()" [disabled]="busy()">Refresh all permitted</button>
    </oc-page-header>
    <p class="notice notice--warn"><oc-term key="coverage_gap">Blocked means coverage gap</oc-term>: a blocked, stale or disabled source means results may be incomplete. It never means there are no settlements. Save the page from your own browser and import the snapshot below.</p>

    <div class="table-wrap card">
      <table>
        <thead><tr><th scope="col">Source</th><th scope="col">Health</th><th scope="col">Last OK</th><th scope="col">Next fetch</th><th scope="col">Failures</th><th scope="col">Actions</th></tr></thead>
        <tbody>
          @for (s of sources(); track s.id) {
            <tr [attr.data-source-id]="s.id">
              <td><button class="btn btn--sm btn--ghost" type="button" (click)="select(s)" [attr.aria-expanded]="selected()?.id === s.id">{{ s.name }}</button><div class="small faint">{{ s.id }} · tier {{ s.authority_tier }} · {{ s.trust_status }}</div></td>
              <td><oc-badge [value]="s.enabled ? s.health : 'disabled'" />@if (s.disabled_reason) { <div class="small muted">{{ s.disabled_reason }}</div> }</td>
              <td>{{ s.last_ok_at ? relative(s.last_ok_at) : 'never' }}</td>
              <td>{{ s.next_fetch_at ? relative(s.next_fetch_at) : '—' }}</td>
              <td>{{ s.consecutive_failures }}@if (s.last_failure) { <div class="small muted">{{ humanize(s.last_failure) }}: {{ s.last_failure_detail }}</div> }</td>
              <td class="row">
                <button class="btn btn--sm" type="button" (click)="refresh(s.id)" [disabled]="busy() || !s.enabled">Refresh</button>
                @if (s.enabled) { <button class="btn btn--sm btn--ghost" type="button" (click)="disable(s)" [disabled]="busy()">Disable</button> } @else { <button class="btn btn--sm" type="button" (click)="enable(s.id)" [disabled]="busy()">Enable</button> }
              </td>
            </tr>
          }
        </tbody>
      </table>
    </div>

    @if (selected(); as s) {
      <section class="section card" aria-labelledby="src-h">
        <div class="row row--between"><h2 id="src-h">{{ s.name }}</h2><a [href]="s.url" target="_blank" rel="noopener noreferrer">Official page ↗</a></div>
        <details><summary>Manifest</summary><pre class="mono small">{{ manifest() }}</pre></details>
        <h3>Snapshot history</h3>
        <ul class="plain">
          @for (h of history(); track h.id) {
            <li><span class="small">{{ date(h.fetched_at) }}</span> · {{ humanize(h.acquisition) }} · {{ h.change_summary || '' }} <div class="small faint mono">{{ h.url }} · {{ h.content_hash.slice(0, 12) }} · parser {{ h.parser_version }}</div></li>
          } @empty { <li class="muted">No snapshots.</li> }
        </ul>
      </section>
    }

    <section class="section card" aria-labelledby="snap-h">
      <h2 id="snap-h">Import a page snapshot</h2>
      <p class="hint">When a site blocks automated access, open it in your own browser, view the page source, and paste the HTML here with its URL. The daemon parses it with the same deterministic parser and records the acquisition as a user snapshot.</p>
      <form (submit)="importSnapshot($event)">
        <div class="row">
          <div class="field" style="margin:0"><label for="snap-src">Source</label><select id="snap-src" [value]="snapSource()" (change)="snapSource.set($any($event.target).value)">@for (s of sources(); track s.id) { <option [value]="s.id">{{ s.name }}</option> }</select></div>
          <div class="field" style="margin:0; flex:1"><label for="snap-url">Page URL</label><input id="snap-url" type="text" [value]="snapUrl()" (input)="snapUrl.set($any($event.target).value)" placeholder="https://…" required /></div>
        </div>
        <div class="field"><label for="snap-html">Page HTML</label><textarea id="snap-html" [value]="snapHtml()" (input)="snapHtml.set($any($event.target).value)" required></textarea></div>
        <button class="btn btn--primary" type="submit" [disabled]="!snapSource() || !snapUrl().trim() || !snapHtml().trim() || busy()">Import snapshot</button>
      </form>
      @if (snapResult()) { <p class="notice notice--ok" style="margin-top:0.75rem">{{ snapResult() }}</p> }
    </section>
  `,
})
export class SourcesPage implements OnInit {
  private readonly api = inject(ApiService);
  private readonly toast = inject(ToastService);
  private readonly confirm = inject(ConfirmService);
  private readonly route = inject(ActivatedRoute);

  readonly sources = signal<Source[]>([]);
  readonly selected = signal<Source | null>(null);
  readonly manifest = signal('');
  readonly history = signal<HistoryRow[]>([]);
  readonly busy = signal(false);
  readonly snapSource = signal('');
  readonly snapUrl = signal('');
  readonly snapHtml = signal('');
  readonly snapResult = signal('');
  readonly humanize = humanize;
  readonly date = date;
  readonly relative = relative;

  async ngOnInit(): Promise<void> {
    await this.reload();
    const pre = this.route.snapshot.queryParamMap.get('source');
    const s = this.sources().find((x) => x.id === pre);
    if (s) await this.select(s);
  }

  async reload(): Promise<void> {
    try {
      this.sources.set((await this.api.op<Paged<Source>>('sources.list', {})).items ?? []);
      if (!this.snapSource()) this.snapSource.set(this.sources()[0]?.id ?? '');
    } catch (e) { this.toast.error(e); }
  }

  async select(s: Source): Promise<void> {
    if (this.selected()?.id === s.id) { this.selected.set(null); return; }
    this.selected.set(s);
    try {
      const [show, hist] = await Promise.all([this.api.op<{ manifest: unknown }>('sources.show', { source_id: s.id }), this.api.op<Paged<HistoryRow>>('sources.history', { source_id: s.id, limit: 20 })]);
      this.manifest.set(JSON.stringify(show.manifest, null, 2));
      this.history.set(hist.items ?? []);
    } catch (e) { this.toast.error(e); }
  }

  private async run(fn: () => Promise<void>): Promise<void> {
    this.busy.set(true);
    try { await fn(); await this.reload(); } catch (e) { this.toast.error(e); } finally { this.busy.set(false); }
  }

  refresh(id: string): Promise<void> { return this.run(async () => { await this.api.op('sources.refresh', { source_id: id }); this.toast.ok('Refresh queued (see Agent Activity → Jobs).'); }); }
  refreshAll(): Promise<void> { return this.run(async () => { await this.api.op('sources.refresh', { scope: 'all' }); this.toast.ok('Refresh jobs queued.'); }); }
  enable(id: string): Promise<void> { return this.run(async () => { await this.api.op('sources.enable', { source_id: id }); }); }
  async disable(s: Source): Promise<void> {
    if (!(await this.confirm.ask({ title: `Disable ${s.name}?`, message: 'No further fetches will run. Previously verified data is kept with a freshness warning.', confirmLabel: 'Disable' }))) return;
    await this.run(async () => { await this.api.op('sources.disable', { source_id: s.id, reason: 'disabled from GUI' }); });
  }

  importSnapshot(ev: Event): Promise<void> {
    ev.preventDefault();
    return this.run(async () => {
      const r = await this.api.op<{ parsed: number; summary: { inserted: number; updated: number; changes: number } }>('sources.import_snapshot', { source_id: this.snapSource(), url: this.snapUrl().trim(), html: this.snapHtml() });
      this.snapResult.set(`Parsed ${r.parsed} record(s): ${r.summary.inserted} new, ${r.summary.updated} updated, ${r.summary.changes} change(s).`);
      this.snapHtml.set('');
    });
  }
}
