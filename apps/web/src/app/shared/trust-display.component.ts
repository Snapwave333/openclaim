import { ChangeDetectionStrategy, Component, computed, inject, input, output, signal } from '@angular/core';
import { ApiService } from '../core/api.service';
import { Trust } from '../core/api.types';
import { date, dateOnly } from '../core/format';
import { ToastService } from '../core/toast.service';
import { BadgeComponent } from './badge.component';
import { TermComponent } from './term.component';

/**
 * Trust display shown on every opportunity card and detail (MDR §10):
 * Primary source, Verified on, Claim deadline (+verified?), eligibility note, verification badge,
 * plus the persistent "View original source" and "Mark uncertain" actions.
 */
@Component({
  selector: 'oc-trust-display',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [BadgeComponent, TermComponent],
  template: `
    <dl class="trust" aria-label="Trust information">
      <div>
        <dt><oc-term key="primary_source">Primary source</oc-term></dt>
        <dd data-field="primary_source">{{ trust().primary_source ? 'Yes' : 'No' }}</dd>
      </div>
      <div>
        <dt>Verified on</dt>
        <dd data-field="verified_on">{{ verifiedOn() }}</dd>
      </div>
      <div>
        <dt><oc-term key="claim_deadline">Claim deadline</oc-term></dt>
        <dd data-field="claim_deadline">
          {{ deadline() }}
          @if (trust().claim_deadline) {
            <span class="faint">({{ trust().deadline_verified ? 'verified' : 'not verified' }})</span>
          }
        </dd>
      </div>
      <div>
        <dt>Verification</dt>
        <dd><oc-badge [value]="trust().verification_state" /></dd>
      </div>
    </dl>
    <p class="trust__note" data-field="eligibility_note"><span aria-hidden="true">!</span> {{ trust().eligibility_note || 'Eligibility still requires review' }}</p>
    <div class="trust__actions">
      @if (officialUrl()) {
        <a class="btn btn--sm" [href]="officialUrl()" target="_blank" rel="noopener noreferrer" data-action="view-source">View original source ↗</a>
      }
      @if (reporting()) {
        <form class="row" (submit)="submitReport($event)">
          <label class="visually-hidden" [attr.for]="'reason-' + opportunityId()">Why is this uncertain?</label>
          <input [id]="'reason-' + opportunityId()" type="text" [value]="reason()" (input)="reason.set($any($event.target).value)" placeholder="Why is this uncertain?" required style="width: 16rem" />
          <button class="btn btn--sm btn--primary" type="submit" [disabled]="!reason().trim() || busy()">Send</button>
          <button class="btn btn--sm btn--ghost" type="button" (click)="reporting.set(false)">Cancel</button>
        </form>
      } @else {
        <button class="btn btn--sm" type="button" (click)="reporting.set(true)" data-action="mark-uncertain">Mark uncertain</button>
      }
    </div>
  `,
})
export class TrustDisplayComponent {
  private readonly api = inject(ApiService);
  private readonly toast = inject(ToastService);

  readonly trust = input.required<Trust>();
  readonly officialUrl = input<string | null | undefined>(null);
  readonly opportunityId = input.required<string>();
  readonly reported = output<string>();

  readonly reporting = signal(false);
  readonly reason = signal('');
  readonly busy = signal(false);

  readonly verifiedOn = computed(() => date(this.trust().verified_on));
  readonly deadline = computed(() => this.trust().claim_deadline ? dateOnly(this.trust().claim_deadline) : 'Unknown');

  async submitReport(ev: Event): Promise<void> {
    ev.preventDefault();
    const reason = this.reason().trim();
    if (!reason) return;
    this.busy.set(true);
    try {
      await this.api.op('opportunities.report', { id: this.opportunityId(), reason });
      this.toast.ok('Marked as uncertain. Thank you.');
      this.reporting.set(false);
      this.reason.set('');
      this.reported.emit(reason);
    } catch (e) {
      this.toast.error(e);
    } finally {
      this.busy.set(false);
    }
  }
}
