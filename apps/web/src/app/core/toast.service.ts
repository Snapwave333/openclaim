import { Injectable, signal } from '@angular/core';
import { ApiError } from './api.types';

export interface Toast { id: number; kind: 'info' | 'ok' | 'error'; text: string }

@Injectable({ providedIn: 'root' })
export class ToastService {
  readonly items = signal<Toast[]>([]);
  private seq = 0;

  show(text: string, kind: Toast['kind'] = 'info', ttlMs = 6000): void {
    const id = ++this.seq;
    this.items.update((l) => [...l, { id, kind, text }]);
    if (ttlMs > 0) setTimeout(() => this.dismiss(id), ttlMs);
  }
  ok(text: string): void { this.show(text, 'ok'); }
  error(e: unknown): void {
    const text = e instanceof ApiError ? `${e.code}: ${e.message}${e.body?.next_action ? ` — ${e.body.next_action}` : ''}` : e instanceof Error ? e.message : String(e);
    this.show(text, 'error', 10000);
  }
  dismiss(id: number): void { this.items.update((l) => l.filter((t) => t.id !== id)); }
}
