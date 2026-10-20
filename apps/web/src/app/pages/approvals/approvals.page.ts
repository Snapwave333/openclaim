import { ChangeDetectionStrategy, Component, OnDestroy, OnInit, computed, inject, signal } from '@angular/core';
import { Subscription } from 'rxjs';
import { ApiService } from '../../core/api.service';
import { Approval, Paged } from '../../core/api.types';
import { EventsService } from '../../core/events.service';
import { date, relative } from '../../core/format';
import { ToastService } from '../../core/toast.service';
import { BadgeComponent } from '../../shared/badge.component';
import { PageHeaderComponent } from '../../shared/page-header.component';
import { TermComponent } from '../../shared/term.component';

@Component({
  selector: 'oc-approvals',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [BadgeComponent, PageHeaderComponent, TermComponent],
  template: `
    <oc-page-header title="Approvals" subtitle="Agents may ask for actions outside their scope. Nothing happens until you approve; approvals expire and never auto-approve." />

    <section aria-labelledby="pend-h">
      <h2 id="pend-h">Pending ({{ pending().length }})</h2>
      @if (pending().length === 0) { <p class="card muted">No pending approvals.</p> }
      <div class="stack">
        @for (a of pending(); track a.id) {
          <article class="card agent-proposed" [attr.data-approval-id]="a.id">
            <span class="agent-proposed__label">Agent proposed</span>
            <div class="row row--between">
              <h3 style="margin:0"><code>{{ a.op }}</code></h3>
              <span class="small muted">requested {{ relative(a.requested_at) }}@if (a.expires_at) { · expires {{ relative(a.expires_at) }} }</span>
            </div>
            <p>{{ a.summary || 'No summary provided.' }} <span class="small muted">— by {{ a.agent_name || a.agent_id || 'unknown agent' }}</span></p>
            <details><summary>Parameters (redacted preview)</summary><pre class="mono small">{{ pretty(a.params_redacted ?? a.params) }}</pre></details>
            <p class="small"><oc-term key="action_digest">Action digest</oc-term>: <code>{{ a.action_digest }}</code></p>
            <div class="field"><label [for]="'note-' + a.id">Note (optional)</label><input [id]="'note-' + a.id" type="text" [value]="notes()[a.id] ?? ''" (input)="setNote(a.id, $any($event.target).value)" /></div>
            <div class="row">
              <button class="btn btn--primary" type="button" (click)="decide(a, 'approve')" [disabled]="busy()">Approve</button>
              <button class="btn btn--danger" type="button" (click)="decide(a, 'deny')" [disabled]="busy()">Deny</button>
            </div>
          </article>
        }
      </div>
    </section>

    <section class="section" aria-labelledby="dec-h">
      <h2 id="dec-h">Decided</h2>
      <div class="table-wrap card">
        <table>
          <thead><tr><th scope="col">Operation</th><th scope="col">Agent</th><th scope="col">Status</th><th scope="col">Requested</th><th scope="col">Decided</th><th scope="col">Note</th></tr></thead>
          <tbody>
            @for (a of decided(); track a.id) {
              <tr><td><code>{{ a.op }}</code></td><td>{{ a.agent_name || a.agent_id || '—' }}</td><td><oc-badge [value]="a.status" /></td><td class="small">{{ date(a.requested_at) }}</td><td class="small">{{ date(a.decided_at) }}</td><td class="small">{{ a.note || '' }}</td></tr>
            } @empty { <tr><td colspan="6" class="muted">No decided approvals.</td></tr> }
          </tbody>
        </table>
      </div>
    </section>
  `,
})
export class ApprovalsPage implements OnInit, OnDestroy {
  private readonly api = inject(ApiService);
  private readonly toast = inject(ToastService);
  private readonly events = inject(EventsService);
  private sub?: Subscription;

  readonly items = signal<Approval[]>([]);
  readonly notes = signal<Record<string, string>>({});
  readonly busy = signal(false);
  readonly pending = computed(() => this.items().filter((a) => a.status === 'pending'));
  readonly decided = computed(() => this.items().filter((a) => a.status !== 'pending'));
  readonly date = date;
  readonly relative = relative;

  async ngOnInit(): Promise<void> {
    await this.reload();
    this.sub = this.events.events$.subscribe((e) => { if (e.topic.startsWith('approval.')) void this.reload(); });
  }
  ngOnDestroy(): void { this.sub?.unsubscribe(); }

  async reload(): Promise<void> {
    try {
      const r = await this.api.op<Paged<Approval>>('approvals.list', { limit: 100 });
      this.items.set(r.items ?? []);
      this.events.pendingApprovals.set(this.pending().length);
    } catch (e) { this.toast.error(e); }
  }

  setNote(id: string, v: string): void { this.notes.update((n) => ({ ...n, [id]: v })); }

  async decide(a: Approval, decision: 'approve' | 'deny'): Promise<void> {
    this.busy.set(true);
    try {
      // The digest shown to the user is sent back; the daemon rejects the decision if the request changed.
      await this.api.post(`/agents/approvals/${encodeURIComponent(a.id)}/decision`, { decision, digest: a.action_digest, note: this.notes()[a.id] || null });
      this.toast.ok(decision === 'approve' ? 'Approved.' : 'Denied.');
      await this.reload();
    } catch (e) { this.toast.error(e); } finally { this.busy.set(false); }
  }

  pretty(v: unknown): string { return v === undefined ? '{}' : JSON.stringify(v, null, 2); }
}
