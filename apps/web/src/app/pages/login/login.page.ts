import { ChangeDetectionStrategy, Component, OnInit, computed, inject, signal } from '@angular/core';
import { ActivatedRoute, Router } from '@angular/router';
import { AuthService } from '../../core/auth.service';

@Component({
  selector: 'oc-login',
  changeDetection: ChangeDetectionStrategy.OnPush,
  template: `
    <main class="login">
      <section class="card" aria-labelledby="login-title">
        <div class="brand" style="padding-left:0">OPEN<span>CLAIM</span></div>
        @if (!auth.checked()) {
          <p class="muted">Contacting the local daemon…</p>
        } @else if (auth.initialized() === null) {
          <h1 id="login-title">Daemon not reachable</h1>
          <p class="muted">The OpenClaim daemon (<code>openclaimd</code>) is not answering on localhost. Start it and reload this page.</p>
          <p class="small">{{ auth.error() }}</p>
          <button class="btn btn--primary" type="button" (click)="auth.refresh()">Retry</button>
        } @else if (auth.initialized() === false) {
          <h1 id="login-title">First run: set a passphrase</h1>
          <p class="muted">This passphrase protects your local data and gates approvals, personal data and this GUI. OpenClaim cannot recover it for you.</p>
          <form (submit)="setup($event)">
            <div class="field">
              <label for="pp1">Passphrase</label>
              <input id="pp1" type="password" autocomplete="new-password" [value]="pass()" (input)="pass.set($any($event.target).value)" required minlength="8" />
            </div>
            <div class="field">
              <label for="pp2">Repeat passphrase</label>
              <input id="pp2" type="password" autocomplete="new-password" [value]="pass2()" (input)="pass2.set($any($event.target).value)" required />
            </div>
            @if (mismatch()) { <p class="small" style="color: var(--danger)" role="alert">Passphrases do not match.</p> }
            @if (auth.error()) { <p class="small" style="color: var(--danger)" role="alert">{{ auth.error() }}</p> }
            <button class="btn btn--primary" type="submit" [disabled]="auth.busy() || mismatch() || pass().length < 8">Create and unlock</button>
          </form>
        } @else {
          <h1 id="login-title">Unlock OpenClaim</h1>
          @if (reason() === 'expired') { <p class="notice notice--warn" role="status">Your session ended. Unlock again to continue.</p> }
          <p class="muted">Everything stays on this computer. No account, no cloud, no AI.</p>
          <form (submit)="login($event)">
            <div class="field">
              <label for="pp">Passphrase</label>
              <input id="pp" type="password" autocomplete="current-password" [value]="pass()" (input)="pass.set($any($event.target).value)" required autofocus />
            </div>
            @if (auth.error()) { <p class="small" style="color: var(--danger)" role="alert">{{ auth.error() }}</p> }
            <button class="btn btn--primary" type="submit" [disabled]="auth.busy() || !pass()">Unlock</button>
          </form>
        }
      </section>
    </main>
  `,
})
export class LoginPage implements OnInit {
  readonly auth = inject(AuthService);
  private readonly router = inject(Router);
  private readonly route = inject(ActivatedRoute);

  readonly pass = signal('');
  readonly pass2 = signal('');
  readonly reason = signal<string | null>(null);
  readonly mismatch = computed(() => this.pass2().length > 0 && this.pass() !== this.pass2());

  async ngOnInit(): Promise<void> {
    this.reason.set(this.route.snapshot.queryParamMap.get('reason'));
    if (!this.auth.checked()) await this.auth.refresh();
    if (this.auth.loggedIn()) await this.router.navigate(['/']);
  }

  async login(ev: Event): Promise<void> {
    ev.preventDefault();
    if (await this.auth.login(this.pass())) {
      this.pass.set('');
      await this.router.navigate(['/']);
    }
  }

  async setup(ev: Event): Promise<void> {
    ev.preventDefault();
    if (this.mismatch()) return;
    if (await this.auth.init(this.pass())) {
      this.pass.set('');
      this.pass2.set('');
      await this.router.navigate(['/']);
    }
  }
}
