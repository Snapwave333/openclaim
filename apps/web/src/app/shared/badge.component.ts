import { ChangeDetectionStrategy, Component, computed, input } from '@angular/core';
import { GLOSSARY } from '../core/glossary';
import { humanize } from '../core/format';

type Tone = 'ok' | 'warn' | 'danger' | 'info' | 'accent' | 'pink' | 'neutral';

const TONES: Record<string, Tone> = {
  verified: 'ok', healthy: 'ok', succeeded: 'ok', extracted: 'ok', approved: 'ok', paid: 'ok', potential_match: 'ok', running: 'info',
  verification_pending: 'warn', stale: 'warn', needs_more_information: 'warn', manual_review: 'warn', pending: 'warn', queued: 'info', degraded: 'warn', never_fetched: 'neutral', needs_configuration: 'warn', proof_needed: 'warn',
  disputed: 'danger', blocked: 'danger', policy_blocked: 'danger', failed: 'danger', denied: 'danger', unlikely: 'danger', expired: 'danger', revoked: 'danger', error: 'danger',
  not_applicable: 'neutral', cancelled: 'neutral', withdrawn: 'neutral', disabled: 'neutral',
  submitted: 'accent', ready: 'accent', open: 'accent', active: 'accent', agent_proposed: 'pink',
};

const ICONS: Record<Tone, string> = { ok: '✓', warn: '!', danger: '✕', info: 'i', accent: '•', pink: '◆', neutral: '–' };

/** Color-independent status badge: icon + text, glossary tooltip when known. */
@Component({
  selector: 'oc-badge',
  changeDetection: ChangeDetectionStrategy.OnPush,
  template: `<span class="badge" [class]="'badge badge--' + tone()" [attr.title]="help()" [attr.aria-label]="label() + (help() ? '. ' + help() : '')"><span aria-hidden="true">{{ icon() }}</span>{{ label() }}</span>`,
})
export class BadgeComponent {
  readonly value = input.required<string | null | undefined>();
  readonly text = input<string | undefined>(undefined);
  readonly toneOverride = input<Tone | undefined>(undefined);

  readonly tone = computed<Tone>(() => this.toneOverride() ?? TONES[this.value() ?? ''] ?? 'neutral');
  readonly icon = computed(() => ICONS[this.tone()]);
  readonly label = computed(() => this.text() ?? humanize(this.value() ?? 'unknown'));
  readonly help = computed(() => GLOSSARY[this.value() ?? ''] ?? null);
}
