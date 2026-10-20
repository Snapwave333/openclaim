import { ChangeDetectionStrategy, Component, OnInit, inject, signal } from '@angular/core';
import { RouterLink } from '@angular/router';
import { ApiService } from '../../core/api.service';
import { Claim, Paged } from '../../core/api.types';
import { date, dateOnly, money } from '../../core/format';
import { ToastService } from '../../core/toast.service';
import { BadgeComponent } from '../../shared/badge.component';
import { PageHeaderComponent } from '../../shared/page-header.component';

interface LedgerTotal { currency?: string; status?: string; amount_minor?: number; total_minor?: number; n?: number; [k: string]: unknown }

@Component({
  selector: 'oc-claims',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [RouterLink, BadgeComponent, PageHeaderComponent],
  template: `
    <oc-page-header title="Claims" subtitle="Claims you chose to track. OpenClaim prepares and records; you file on the official site." />
    @if (loading()) { <p class="muted" role="status">Loading…</p> }
    @if (!loading() && items().length === 0) {
      <p class="card muted">No claims tracked yet. Start one from an opportunity's "Official filing" tab in <a routerLink="/discover">Discover</a>.</p>
    }
    <div class="table-wrap card" [hidden]="items().length === 0">
      <table>
        <thead><tr><th scope="col">Opportunity</th><th scope="col">Person</th><th scope="col">Status</th><th scope="col">Deadline</th><th scope="col">Confirmation</th><th scope="col">Updated</th></tr></thead>
        <tbody>
          @for (c of items(); track c.id) {
            <tr>
              <td><a [routerLink]="['/claims', c.id]">{{ c.opportunity_title }}</a></td>
              <td>{{ c.display_name }}</td>
              <td><oc-badge [value]="c.status" /></td>
              <td>{{ c.claim_deadline ? dateOnly(c.claim_deadline) + (c.deadline_verified ? ' (verified)' : ' (not verified)') : 'unknown' }}</td>
              <td class="mono small">{{ c.external_confirmation_id || '—' }}</td>
              <td class="small muted">{{ date(c.updated_at) }}</td>
            </tr>
          }
        </tbody>
      </table>
    </div>

    <section class="section card" aria-labelledby="ledger-h">
      <h2 id="ledger-h">Ledger</h2>
      <p class="hint">{{ ledgerNote() }}</p>
      @if (ledger().length === 0) { <p class="muted">No recorded payments.</p> }
      <ul class="plain">
        @for (t of ledger(); track $index) {
          <li class="row row--between"><span>{{ t.status || t.currency || 'total' }}</span><span class="mono">{{ money(t.total_minor ?? t.amount_minor ?? 0, t.currency || 'USD') }}{{ t.n ? ' · ' + t.n + ' claim(s)' : '' }}</span></li>
        }
      </ul>
    </section>
  `,
})
export class ClaimsPage implements OnInit {
  private readonly api = inject(ApiService);
  private readonly toast = inject(ToastService);
  readonly loading = signal(true);
  readonly items = signal<Claim[]>([]);
  readonly ledger = signal<LedgerTotal[]>([]);
  readonly ledgerNote = signal('');
  readonly date = date;
  readonly dateOnly = dateOnly;
  readonly money = money;

  async ngOnInit(): Promise<void> {
    try {
      const [list, ledger] = await Promise.all([this.api.op<Paged<Claim>>('claims.list', { limit: 200 }), this.api.op<{ items: LedgerTotal[]; totals: LedgerTotal[]; note: string }>('claims.ledger', {})]);
      this.items.set(list.items ?? []);
      this.ledger.set(ledger.totals?.length ? ledger.totals : ledger.items ?? []);
      this.ledgerNote.set(ledger.note ?? '');
    } catch (e) { this.toast.error(e); } finally { this.loading.set(false); }
  }
}
