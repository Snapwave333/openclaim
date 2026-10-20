import { ChangeDetectionStrategy, Component, computed, input } from '@angular/core';
import { GLOSSARY } from '../core/glossary';

/** Plain-language glossary term with a native tooltip (works with keyboard focus via tabindex). */
@Component({
  selector: 'oc-term',
  changeDetection: ChangeDetectionStrategy.OnPush,
  template: `<span class="term" tabindex="0" [attr.title]="definition()" [attr.aria-description]="definition()"><ng-content /></span>`,
})
export class TermComponent {
  readonly key = input.required<string>();
  readonly definition = computed(() => GLOSSARY[this.key()] ?? '');
}
