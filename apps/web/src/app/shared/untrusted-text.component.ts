import { ChangeDetectionStrategy, Component, input } from '@angular/core';

/**
 * Renders third-party text (evidence excerpts, document text) as PLAIN TEXT in a distinct style.
 * Angular interpolation escapes everything; this component never uses innerHTML.
 */
@Component({
  selector: 'oc-untrusted-text',
  changeDetection: ChangeDetectionStrategy.OnPush,
  template: `
    <div class="untrusted" data-trust="untrusted_source_text" role="group" aria-label="Untrusted source text">
      <span class="untrusted__label">Untrusted source text{{ origin() ? ' · ' + origin() : '' }}</span>{{ text() }}
    </div>
  `,
})
export class UntrustedTextComponent {
  readonly text = input.required<string>();
  readonly origin = input<string | null | undefined>(null);
}
