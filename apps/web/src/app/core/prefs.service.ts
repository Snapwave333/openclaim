import { Injectable, effect, signal } from '@angular/core';

export type Theme = 'dark' | 'light' | 'system';
export type Motion = 'system' | 'reduced' | 'full';

const KEY = 'openclaim.prefs';

interface Prefs { theme: Theme; fontScale: number; motion: Motion }

/** Display preferences (theme, font size, motion). Persisted in localStorage; nothing sensitive. */
@Injectable({ providedIn: 'root' })
export class PrefsService {
  readonly theme = signal<Theme>('system');
  readonly fontScale = signal(1);
  readonly motion = signal<Motion>('system');

  constructor() {
    this.load();
    effect(() => this.apply());
    if (typeof window !== 'undefined' && window.matchMedia) {
      window.matchMedia('(prefers-color-scheme: light)').addEventListener('change', () => this.apply());
    }
  }

  private load(): void {
    try {
      const raw = localStorage.getItem(KEY);
      if (!raw) return;
      const p = JSON.parse(raw) as Partial<Prefs>;
      if (p.theme) this.theme.set(p.theme);
      if (typeof p.fontScale === 'number') this.fontScale.set(Math.min(1.6, Math.max(0.8, p.fontScale)));
      if (p.motion) this.motion.set(p.motion);
    } catch { /* ignore */ }
  }

  resolvedTheme(): 'dark' | 'light' {
    const t = this.theme();
    if (t !== 'system') return t;
    const light = typeof window !== 'undefined' && window.matchMedia?.('(prefers-color-scheme: light)').matches;
    return light ? 'light' : 'dark';
  }

  private apply(): void {
    const prefs: Prefs = { theme: this.theme(), fontScale: this.fontScale(), motion: this.motion() };
    try { localStorage.setItem(KEY, JSON.stringify(prefs)); } catch { /* ignore */ }
    if (typeof document === 'undefined') return;
    const root = document.documentElement;
    root.setAttribute('data-theme', this.resolvedTheme());
    root.style.setProperty('--font-scale', String(prefs.fontScale));
    if (prefs.motion === 'system') root.removeAttribute('data-motion');
    else root.setAttribute('data-motion', prefs.motion);
  }
}
