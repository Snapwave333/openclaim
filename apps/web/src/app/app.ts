import { ChangeDetectionStrategy, Component, effect, inject } from '@angular/core';
import { RouterLink, RouterLinkActive, RouterOutlet } from '@angular/router';
import { AuthService } from './core/auth.service';
import { EventsService } from './core/events.service';
import { PrefsService } from './core/prefs.service';
import { ToastService } from './core/toast.service';

interface NavItem { path: string; label: string; icon: string }

const NAV: NavItem[] = [
  { path: '/', label: 'Overview', icon: '◳' },
  { path: '/discover', label: 'Discover', icon: '⌕' },
  { path: '/matches', label: 'My Matches', icon: '≡' },
  { path: '/claims', label: 'Claims', icon: '✎' },
  { path: '/vault', label: 'Evidence Vault', icon: '▣' },
  { path: '/sources', label: 'Sources', icon: '⊕' },
  { path: '/agents', label: 'Agent Activity', icon: '⚙' },
  { path: '/approvals', label: 'Approvals', icon: '✓' },
  { path: '/settings', label: 'Settings', icon: '⋯' },
];

@Component({
  selector: 'app-root',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [RouterOutlet, RouterLink, RouterLinkActive],
  template: `
    @if (auth.loggedIn()) {
      <a class="skip-link" href="#main">Skip to content</a>
      <div class="shell">
        <nav class="shell__nav" aria-label="Main">
          <div class="brand">OPEN<span>CLAIM</span></div>
          @for (item of nav; track item.path) {
            <a class="navlink" [routerLink]="item.path" routerLinkActive="active" [routerLinkActiveOptions]="{ exact: item.path === '/' }" ariaCurrentWhenActive="page">
              <span aria-hidden="true">{{ item.icon }}</span>{{ item.label }}
              @if (item.path === '/approvals' && events.pendingApprovals() > 0) {
                <span class="navlink__badge" aria-label="pending approvals">{{ events.pendingApprovals() }}</span>
              }
            </a>
          }
          <div class="spacer"></div>
          <p class="hint" style="padding: 0 0.6rem">Live events: {{ events.connected() ? 'connected' : 'reconnecting' }}</p>
          <button class="btn btn--sm btn--ghost" type="button" (click)="auth.logout()">Lock &amp; sign out</button>
        </nav>
        <main id="main" class="shell__main" tabindex="-1">
          <router-outlet />
        </main>
      </div>
    } @else {
      <router-outlet />
    }
    <div class="toasts" aria-live="polite" aria-atomic="false">
      @for (t of toast.items(); track t.id) {
        <div class="toast" [class.toast--error]="t.kind === 'error'" [class.toast--ok]="t.kind === 'ok'" role="status">
          {{ t.text }}
          <button class="btn btn--sm btn--ghost" type="button" (click)="toast.dismiss(t.id)" aria-label="Dismiss notification">×</button>
        </div>
      }
    </div>
  `,
})
export class App {
  readonly auth = inject(AuthService);
  readonly events = inject(EventsService);
  readonly toast = inject(ToastService);
  readonly nav = NAV;

  constructor() {
    inject(PrefsService); // applies the theme on startup
    effect(() => {
      if (this.auth.loggedIn()) this.events.connect();
      else this.events.disconnect();
    });
  }
}
