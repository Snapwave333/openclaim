import { Injectable, computed, effect, inject, signal } from '@angular/core';
import { Router } from '@angular/router';
import { ApiService } from './api.service';
import { ApiError } from './api.types';

/** Owner session state for the GUI. The CSRF token stays inside ApiService memory. */
@Injectable({ providedIn: 'root' })
export class AuthService {
  private readonly api = inject(ApiService);
  private readonly router = inject(Router);

  readonly checked = signal(false);
  readonly initialized = signal<boolean | null>(null);
  readonly unlocked = signal(false);
  readonly busy = signal(false);
  readonly error = signal<string | null>(null);

  /** Logged-in means the daemon says unlocked AND we hold a csrf token for mutations. */
  readonly loggedIn = computed(() => this.unlocked() && this.api.hasCsrf());

  constructor() {
    effect(() => {
      if (this.api.sessionLost()) {
        this.unlocked.set(false);
        this.api.setCsrf(null);
        void this.router.navigate(['/login'], { queryParams: { reason: 'expired' } });
      }
    });
  }

  async refresh(): Promise<void> {
    try {
      const me = await this.api.me();
      this.initialized.set(me.initialized);
      // Fresh page load: the HttpOnly cookie may still be valid; re-issue the in-memory CSRF token.
      if (me.unlocked && !this.api.hasCsrf()) {
        await this.api.refreshCsrf();
      }
      this.unlocked.set(me.unlocked && this.api.hasCsrf());
    } catch (e) {
      this.error.set(e instanceof ApiError ? e.message : 'daemon unreachable');
      this.initialized.set(null);
      this.unlocked.set(false);
    } finally {
      this.checked.set(true);
    }
  }

  async login(passphrase: string): Promise<boolean> {
    this.busy.set(true);
    this.error.set(null);
    try {
      await this.api.login(passphrase);
      this.unlocked.set(true);
      this.initialized.set(true);
      return true;
    } catch (e) {
      this.error.set(e instanceof ApiError ? e.message : String(e));
      return false;
    } finally {
      this.busy.set(false);
    }
  }

  async init(passphrase: string): Promise<boolean> {
    this.busy.set(true);
    this.error.set(null);
    try {
      await this.api.op('owner.init', { passphrase });
      this.initialized.set(true);
      return this.login(passphrase);
    } catch (e) {
      this.error.set(e instanceof ApiError ? e.message : String(e));
      return false;
    } finally {
      this.busy.set(false);
    }
  }

  async logout(): Promise<void> {
    try { await this.api.logout(); } catch { /* cookie is cleared regardless */ }
    this.unlocked.set(false);
    await this.router.navigate(['/login']);
  }
}
