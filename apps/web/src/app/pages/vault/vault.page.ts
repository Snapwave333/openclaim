import { ChangeDetectionStrategy, Component, OnDestroy, OnInit, inject, signal } from '@angular/core';
import { Subscription } from 'rxjs';
import { ApiService } from '../../core/api.service';
import { Claim, Doc, Paged, Suggestion, UntrustedText } from '../../core/api.types';
import { EventsService } from '../../core/events.service';
import { date, humanize, money } from '../../core/format';
import { ToastService } from '../../core/toast.service';
import { BadgeComponent } from '../../shared/badge.component';
import { ConfirmService } from '../../shared/confirm.service';
import { PageHeaderComponent } from '../../shared/page-header.component';
import { PeopleService } from '../../shared/people.service';
import { PersonSelectComponent } from '../../shared/person-select.component';
import { UntrustedTextComponent } from '../../shared/untrusted-text.component';

interface SearchHit { id: string; doc_type: string; extraction_state: string; person_id: string | null; imported_at: string; snippet: string }

@Component({
  selector: 'oc-vault',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [BadgeComponent, PageHeaderComponent, PersonSelectComponent, UntrustedTextComponent],
  template: `
    <oc-page-header title="Evidence Vault" subtitle="Receipts, notices and statements stay encrypted on this computer. Extracted text is treated as untrusted data." />

    <div class="grid grid--2">
      <section class="card" aria-labelledby="imp-h">
        <h2 id="imp-h">Import a document</h2>
        <p class="hint">A browser cannot read files from your disk by path, so enter the full absolute path of a file already on this computer (for example <code>C:\\Users\\you\\Documents\\receipt.pdf</code>). The daemon copies it into the vault and runs local extraction.</p>
        <form (submit)="importDoc($event)">
          <div class="field"><label for="imp-path">Absolute file path</label><input id="imp-path" type="text" [value]="path()" (input)="path.set($any($event.target).value)" required spellcheck="false" /></div>
          <oc-person-select />
          <button class="btn btn--primary" type="submit" [disabled]="!path().trim() || busy()">Import</button>
        </form>
      </section>
      <section class="card" aria-labelledby="srch-h">
        <h2 id="srch-h">Search documents</h2>
        <form class="row" (submit)="search($event)">
          <div class="field" style="flex:1; margin:0"><label for="doc-q">Text search (local FTS)</label><input id="doc-q" type="search" [value]="q()" (input)="q.set($any($event.target).value)" /></div>
          <button class="btn" type="submit" [disabled]="!q().trim()" style="align-self:flex-end">Search</button>
        </form>
        <ul class="plain">
          @for (h of hits(); track h.id) {
            <li><button class="btn btn--sm btn--ghost" type="button" (click)="open(h.id)">Open</button> <span class="small">{{ docName(h.id) }}</span><div class="small untrusted" style="margin-top:0.3rem">{{ h.snippet }}</div></li>
          }
        </ul>
      </section>
    </div>

    <section class="section card" aria-labelledby="docs-h">
      <h2 id="docs-h">Documents</h2>
      <div class="table-wrap">
        <table>
          <thead><tr><th scope="col">Name</th><th scope="col">Type</th><th scope="col">Classification</th><th scope="col">Extraction</th><th scope="col">Suggestions</th><th scope="col">Imported</th><th scope="col">Actions</th></tr></thead>
          <tbody>
            @for (d of docs(); track d.id) {
              <tr [attr.aria-selected]="selected()?.id === d.id">
                <td>{{ d.name }}<div class="small faint">{{ sizeKb(d.size_bytes) }}</div></td>
                <td>{{ d.doc_type }}</td>
                <td>{{ humanize(d.classification) }}</td>
                <td><oc-badge [value]="d.extraction_state" />@if (d.extraction_note) { <div class="small muted">{{ d.extraction_note }}</div> }</td>
                <td>{{ d.suggestion_count }}</td>
                <td class="small muted">{{ date(d.imported_at) }}</td>
                <td class="row">
                  <button class="btn btn--sm" type="button" (click)="open(d.id)">Open</button>
                  <button class="btn btn--sm btn--danger" type="button" (click)="remove(d)">Delete</button>
                </td>
              </tr>
            } @empty { <tr><td colspan="7" class="muted">No documents yet.</td></tr> }
          </tbody>
        </table>
      </div>
    </section>

    @if (selected(); as d) {
      <section class="section card" aria-labelledby="doc-h">
        <div class="row row--between"><h2 id="doc-h">{{ d.name }}</h2><button class="btn btn--sm btn--ghost" type="button" (click)="selected.set(null)">Close</button></div>
        <div class="grid grid--2">
          <div>
            <h3>Suggestions</h3>
            <p class="hint">Pattern-based suggestions with source spans. Confirm only if true; a confirmed suggestion becomes a human-confirmed fact with this document as proof.</p>
            @for (s of suggestions(); track $index) {
              <div class="agent-proposed" style="margin-bottom:0.5rem">
                <span class="agent-proposed__label">Proposed · not yet confirmed</span>
                <div class="row row--between">
                  <span><strong>{{ humanize(s.kind) }}</strong>: {{ display(s) }}</span>
                  <button class="btn btn--sm" type="button" (click)="confirmFact(s)" [disabled]="busy() || !d.person_id">Confirm as fact</button>
                </div>
                <div class="small untrusted" style="margin-top:0.3rem">{{ s.excerpt }}</div>
              </div>
            } @empty { <p class="muted">No suggestions.</p> }
            <h3 style="margin-top:1rem">Link to a claim</h3>
            <form class="row" (submit)="link($event)">
              <select [value]="linkClaim()" (change)="linkClaim.set($any($event.target).value)" aria-label="Claim" style="flex:1"><option value="">Choose a claim…</option>@for (c of claims(); track c.id) { <option [value]="c.id">{{ c.opportunity_title }} ({{ c.display_name }})</option> }</select>
              <button class="btn btn--sm" type="submit" [disabled]="!linkClaim() || busy()">Link</button>
            </form>
            <ul class="small muted">@for (l of d.links; track $index) { <li>{{ l.target_kind }} {{ l.target_id.slice(0, 8) }}… {{ l.validated ? '(validated)' : '' }}</li> }</ul>
          </div>
          <div>
            <h3>Extracted text</h3>
            @if (excerpt(); as ex) {
              <oc-untrusted-text [text]="ex.text" [origin]="ex.origin" />
              <div class="row" style="margin-top:0.5rem">
                <button class="btn btn--sm" type="button" (click)="loadExcerpt(d.id, offset() - 2000)" [disabled]="offset() <= 0">← Previous</button>
                <span class="small muted">chars {{ offset() }}–{{ excerptEnd() }} of {{ total() }}</span>
                <button class="btn btn--sm" type="button" (click)="loadExcerpt(d.id, offset() + 2000)" [disabled]="excerptEnd() >= total()">Next →</button>
              </div>
            } @else { <p class="muted">No text extracted{{ d.extraction_state === 'pending' || d.extraction_state === 'running' ? ' yet' : '' }}.</p> }
          </div>
        </div>
      </section>
    }
  `,
})
export class VaultPage implements OnInit, OnDestroy {
  private readonly api = inject(ApiService);
  private readonly toast = inject(ToastService);
  private readonly confirm = inject(ConfirmService);
  private readonly events = inject(EventsService);
  readonly people = inject(PeopleService);
  private sub?: Subscription;

  readonly docs = signal<Doc[]>([]);
  readonly claims = signal<Claim[]>([]);
  readonly selected = signal<Doc | null>(null);
  readonly suggestions = signal<Suggestion[]>([]);
  readonly excerpt = signal<UntrustedText | null>(null);
  readonly offset = signal(0);
  readonly excerptEnd = signal(0);
  readonly total = signal(0);
  readonly hits = signal<SearchHit[]>([]);
  readonly path = signal('');
  readonly q = signal('');
  readonly linkClaim = signal('');
  readonly busy = signal(false);
  readonly humanize = humanize;
  readonly date = date;

  async ngOnInit(): Promise<void> {
    await this.people.load();
    await this.reload();
    try { this.claims.set((await this.api.op<Paged<Claim>>('claims.list', { limit: 200 })).items ?? []); } catch { /* optional */ }
    this.sub = this.events.events$.subscribe((e) => { if (e.topic === 'document.extracted') void this.reload(); });
  }
  ngOnDestroy(): void { this.sub?.unsubscribe(); }

  async reload(): Promise<void> {
    try { this.docs.set((await this.api.op<Paged<Doc>>('documents.list', { limit: 200 })).items ?? []); } catch (e) { this.toast.error(e); }
  }

  docName(id: string): string { return this.docs().find((d) => d.id === id)?.name ?? id.slice(0, 8); }
  sizeKb(b: number): string { return b < 1024 ? `${b} B` : `${(b / 1024).toFixed(1)} KB`; }
  display(s: Suggestion): string { return s.kind.startsWith('amount_minor') ? money(Number(s.value), s.kind.endsWith('usd') ? 'USD' : 'USD') : s.value; }

  async importDoc(ev: Event): Promise<void> {
    ev.preventDefault();
    this.busy.set(true);
    try {
      const r = await this.api.op<{ id: string; duplicate: boolean }>('documents.import', { file: this.path().trim(), person_id: this.people.selectedId() });
      this.toast.ok(r.duplicate ? 'Already in the vault.' : 'Imported; extraction queued.');
      this.path.set('');
      await this.reload();
    } catch (e) { this.toast.error(e); } finally { this.busy.set(false); }
  }

  async open(id: string): Promise<void> {
    try {
      const d = await this.api.op<Doc>('documents.show', { id });
      this.selected.set(d);
      const s = await this.api.op<{ suggestions: Suggestion[] }>('documents.suggestions', { id });
      this.suggestions.set(s.suggestions ?? []);
      await this.loadExcerpt(id, 0);
    } catch (e) { this.toast.error(e); }
  }

  async loadExcerpt(id: string, offset: number): Promise<void> {
    try {
      const r = await this.api.op<{ offset: number; end: number; total_chars: number; text: UntrustedText }>('documents.excerpt', { id, offset: Math.max(0, offset), length: 2000 });
      this.excerpt.set(r.text?.text ? r.text : null);
      this.offset.set(r.offset); this.excerptEnd.set(r.end); this.total.set(r.total_chars);
    } catch { this.excerpt.set(null); }
  }

  async confirmFact(s: Suggestion): Promise<void> {
    const d = this.selected();
    if (!d?.person_id) return;
    this.busy.set(true);
    try {
      const params: Record<string, unknown> = { person_id: d.person_id, fact_type: s.kind === 'merchant' ? 'purchase' : s.kind, positive: true, proof_id: d.id };
      if (s.kind === 'merchant') params['merchant'] = s.value;
      if (s.kind === 'date') { params['start'] = s.value; params['fact_type'] = 'purchase'; }
      if (s.kind.startsWith('amount')) { params['fact_type'] = 'purchase'; params['product'] = `amount ${this.display(s)}`; }
      await this.api.op('facts.add', params);
      this.toast.ok('Fact recorded as human-confirmed with this document as proof.');
    } catch (e) { this.toast.error(e); } finally { this.busy.set(false); }
  }

  async link(ev: Event): Promise<void> {
    ev.preventDefault();
    const d = this.selected();
    if (!d || !this.linkClaim()) return;
    this.busy.set(true);
    try {
      await this.api.op('documents.link', { document_id: d.id, target_kind: 'claim', target_id: this.linkClaim() });
      this.toast.ok('Linked.');
      await this.open(d.id);
      await this.reload();
    } catch (e) { this.toast.error(e); } finally { this.busy.set(false); }
  }

  async remove(d: Doc): Promise<void> {
    if (!(await this.confirm.ask({ title: 'Delete document?', message: `"${d.name}" will be removed from the vault. Facts that used it as proof keep a dangling reference.`, confirmLabel: 'Delete', danger: true }))) return;
    try {
      await this.api.op('documents.delete', { id: d.id, confirm: true });
      if (this.selected()?.id === d.id) this.selected.set(null);
      await this.reload();
    } catch (e) { this.toast.error(e); }
  }

  async search(ev: Event): Promise<void> {
    ev.preventDefault();
    try { this.hits.set((await this.api.op<Paged<SearchHit>>('documents.search', { q: this.q().trim(), limit: 20 })).items ?? []); } catch (e) { this.toast.error(e); }
  }
}
