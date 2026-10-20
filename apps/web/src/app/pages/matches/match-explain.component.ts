import { ChangeDetectionStrategy, Component, inject, input, output, signal } from '@angular/core';
import { ApiService } from '../../core/api.service';
import { MatchExplain } from '../../core/api.types';
import { ToastService } from '../../core/toast.service';
import { BadgeComponent } from '../../shared/badge.component';

/** Explain panel: assessment, cited reasons, missing questions with yes/no attest, "not part of this class". */
@Component({
  selector: 'oc-match-explain',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [BadgeComponent],
  template: `
    <div class="card" data-explain>
      <div class="row row--between">
        <h3 style="margin:0">Assessment: <oc-badge [value]="explain().assessment.state" /></h3>
        <button class="btn btn--sm" type="button" (click)="reevaluate()" [disabled]="busy()">Re-evaluate</button>
      </div>
      <p>{{ explain().assessment.summary }}</p>

      @if (explain().assessment.reasons.length) {
        <h4>Reasons (cited)</h4>
        <ul>
          @for (r of explain().assessment.reasons; track $index) {
            <li>{{ reasonText(r) }}</li>
          }
        </ul>
      }

      @if (explain().assessment.missing_facts.length) {
        <h4>Questions to answer</h4>
        <p class="hint">Answer only what is true. Your answers are stored locally as human-confirmed facts.</p>
        <ul class="plain">
          @for (m of explain().assessment.missing_facts; track $index) {
            <li class="row row--between">
              <span>{{ questionText(m) }}</span>
              <span class="row">
                <button class="btn btn--sm" type="button" (click)="attest(questionKey(m), true)" [disabled]="busy()">Yes</button>
                <button class="btn btn--sm" type="button" (click)="attest(questionKey(m), false)" [disabled]="busy()">No</button>
              </span>
            </li>
          }
        </ul>
      }

      <div class="row" style="margin-top: 0.75rem">
        <button class="btn btn--sm btn--ghost" type="button" (click)="exclude(true)" [disabled]="busy()">I am not part of this class</button>
        @if (explain().assessment.state === 'not_applicable') {
          <button class="btn btn--sm btn--ghost" type="button" (click)="exclude(false)" [disabled]="busy()">Undo exclusion</button>
        }
      </div>
      <p class="hint" style="margin-top: 0.75rem">{{ explain().assessment.disclaimer }}</p>
    </div>
  `,
})
export class MatchExplainComponent {
  private readonly api = inject(ApiService);
  private readonly toast = inject(ToastService);

  readonly explain = input.required<MatchExplain>();
  readonly personId = input.required<string>();
  readonly changed = output<void>();
  readonly busy = signal(false);

  reasonText(r: unknown): string {
    if (typeof r === 'string') return r;
    const o = r as Record<string, unknown>;
    const parts = [o['text'] ?? o['reason'] ?? o['rule'] ?? o['description'], o['result'] ? `(${String(o['result'])})` : null, o['citation'] ?? o['source'] ? `— ${String(o['citation'] ?? o['source'])}` : null];
    const s = parts.filter(Boolean).map(String).join(' ');
    return s || JSON.stringify(r);
  }

  questionText(m: unknown): string {
    if (typeof m === 'string') return m.replace(/_/g, ' ');
    const o = m as Record<string, unknown>;
    return String(o['question'] ?? o['label'] ?? o['prompt'] ?? o['key'] ?? JSON.stringify(m)).replace(/_/g, ' ');
  }

  questionKey(m: unknown): string {
    if (typeof m === 'string') return m;
    const o = m as Record<string, unknown>;
    return String(o['key'] ?? o['fact'] ?? o['id'] ?? '');
  }

  private async run(op: string, params: Record<string, unknown>, okText: string): Promise<void> {
    this.busy.set(true);
    try {
      await this.api.op(op, params);
      this.toast.ok(okText);
      this.changed.emit();
    } catch (e) { this.toast.error(e); } finally { this.busy.set(false); }
  }

  attest(key: string, value: boolean): Promise<void> {
    return this.run('facts.attest', { person_id: this.personId(), opportunity_id: this.explain().opportunity.id, key, value }, 'Answer recorded.');
  }
  exclude(notInClass: boolean): Promise<void> {
    return this.run('facts.exclude', { person_id: this.personId(), opportunity_id: this.explain().opportunity.id, not_in_class: notInClass }, notInClass ? 'Marked as not part of this class.' : 'Exclusion removed.');
  }
  reevaluate(): Promise<void> {
    return this.run('match.evaluate', { person_id: this.personId(), opportunity_id: this.explain().opportunity.id }, 'Re-evaluated.');
  }
}
