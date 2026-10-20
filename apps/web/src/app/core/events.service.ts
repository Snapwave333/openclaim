import { Injectable, NgZone, inject, signal } from '@angular/core';
import { Subject } from 'rxjs';
import { API_BASE } from './api.service';
import { SseEvent } from './api.types';

export const SSE_TOPICS = [
  'job.progress', 'job.completed', 'opportunity.new', 'opportunity.changed', 'alert.due',
  'approval.requested', 'approval.decided', 'source.failed', 'match.changed', 'document.extracted',
] as const;

/** Server-Sent Events from `GET /events`. One EventSource per GUI tab; reconnects automatically. */
@Injectable({ providedIn: 'root' })
export class EventsService {
  private readonly zone = inject(NgZone);
  private source: EventSource | null = null;
  private readonly subject = new Subject<SseEvent>();

  readonly events$ = this.subject.asObservable();
  readonly connected = signal(false);
  readonly lastSeq = signal(0);
  readonly pendingApprovals = signal(0);

  connect(): void {
    if (this.source || typeof EventSource === 'undefined') return;
    const es = new EventSource(`${API_BASE}/events`);
    this.source = es;
    es.onopen = () => this.zone.run(() => this.connected.set(true));
    es.onerror = () => this.zone.run(() => this.connected.set(false));
    for (const topic of SSE_TOPICS) {
      es.addEventListener(topic, (ev: MessageEvent) => {
        let parsed: SseEvent;
        try { parsed = JSON.parse(ev.data) as SseEvent; } catch { return; }
        this.zone.run(() => {
          this.lastSeq.set(parsed.seq);
          if (topic === 'approval.requested') this.pendingApprovals.update((n) => n + 1);
          if (topic === 'approval.decided') this.pendingApprovals.update((n) => Math.max(0, n - 1));
          this.subject.next({ ...parsed, topic });
        });
      });
    }
  }

  disconnect(): void {
    this.source?.close();
    this.source = null;
    this.connected.set(false);
  }
}
