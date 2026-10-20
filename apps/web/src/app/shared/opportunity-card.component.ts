import { ChangeDetectionStrategy, Component, inject, input, output, signal } from '@angular/core';
import { RouterLink } from '@angular/router';
import { ApiService } from '../core/api.service';
import { Opportunity } from '../core/api.types';
import { dateOnly, humanize, money } from '../core/format';
import { ToastService } from '../core/toast.service';
import { BadgeComponent } from './badge.component';
import { TrustDisplayComponent } from './trust-display.component';

@Component({
  selector: 'oc-opportunity-card',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [RouterLink, BadgeComponent, TrustDisplayComponent],
  template: `
    <article class="card opp" [attr.data-opportunity-id]="opp().id" [attr.aria-label]="opp().title">
      <div class="row row--between">
        <a class="opp__title" [routerLink]="['/discover', opp().id]">{{ opp().title }}</a>
        <div class="row">
          @if (compareEnabled()) {
            <label class="field field--inline" style="margin:0"><input type="checkbox" [checked]="compared()" (change)="compareToggle.emit($any($event.target).checked)" /> Compare</label>
          }
          <button class="btn btn--sm" type="button" [attr.aria-pressed]="opp().bookmarked" (click)="bookmark()" [disabled]="busy()" data-action="bookmark">
            {{ opp().bookmarked ? '★ Bookmarked' : '☆ Bookmark' }}
          </button>
          <button class="btn btn--sm btn--ghost" type="button" (click)="hide()" [disabled]="busy()" data-action="hide">{{ opp().hidden ? 'Unhide' : 'Hide' }}</button>
        </div>
      </div>
      <div class="opp__meta">
        <oc-badge [value]="opp().status" />
        <span>{{ humanize(opp().opp_type) }}</span>
        @if (opp().administrator) { <span>· {{ opp().administrator }}</span> }
        @if (opp().jurisdictions.length) { <span>· {{ opp().jurisdictions.join(', ') }}</span> }
        @if (opp().class_period_start || opp().class_period_end) { <span>· Class period {{ dateOnly(opp().class_period_start) }} – {{ dateOnly(opp().class_period_end) }}</span> }
        <span>· Benefit: {{ benefit() }}</span>
      </div>
      @if (opp().summary) { <p class="opp__summary">{{ opp().summary }}</p> }
      <oc-trust-display [trust]="opp().trust" [officialUrl]="opp().official_url" [opportunityId]="opp().id" />
    </article>
  `,
})
export class OpportunityCardComponent {
  private readonly api = inject(ApiService);
  private readonly toast = inject(ToastService);

  readonly opp = input.required<Opportunity>();
  readonly compareEnabled = input(false);
  readonly compared = input(false);
  readonly changed = output<Opportunity>();
  readonly compareToggle = output<boolean>();
  readonly busy = signal(false);

  readonly humanize = humanize;
  readonly dateOnly = dateOnly;

  benefit(): string {
    const b = this.opp().benefit;
    if (!b) return 'unknown';
    if (b.amount_known && typeof b.amount_minor === 'number') return money(b.amount_minor, b.currency ?? 'USD');
    return b.description ?? 'unknown';
  }

  async bookmark(): Promise<void> {
    await this.toggle('opportunities.bookmark', !this.opp().bookmarked, 'bookmarked');
  }
  async hide(): Promise<void> {
    await this.toggle('opportunities.hide', !this.opp().hidden, 'hidden');
  }

  private async toggle(op: string, on: boolean, field: 'bookmarked' | 'hidden'): Promise<void> {
    this.busy.set(true);
    try {
      await this.api.op(op, { id: this.opp().id, on });
      this.changed.emit({ ...this.opp(), [field]: on });
    } catch (e) {
      this.toast.error(e);
    } finally {
      this.busy.set(false);
    }
  }
}
