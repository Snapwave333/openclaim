import { Injectable, inject, signal } from '@angular/core';
import { ApiService } from '../core/api.service';
import { Paged, Person } from '../core/api.types';

const KEY = 'openclaim.selectedPerson';

/** Cached list of local claimant profiles and the person currently selected across screens. */
@Injectable({ providedIn: 'root' })
export class PeopleService {
  private readonly api = inject(ApiService);
  readonly people = signal<Person[]>([]);
  readonly selectedId = signal<string | null>(typeof localStorage !== 'undefined' ? localStorage.getItem(KEY) : null);
  readonly loaded = signal(false);

  async load(force = false): Promise<Person[]> {
    if (this.loaded() && !force) return this.people();
    const data = await this.api.op<Paged<Person>>('people.list', { limit: 200 });
    this.people.set(data.items ?? []);
    this.loaded.set(true);
    const sel = this.selectedId();
    if (!sel || !this.people().some((p) => p.id === sel)) this.select(this.people()[0]?.id ?? null);
    return this.people();
  }

  select(id: string | null): void {
    this.selectedId.set(id);
    try { if (id) localStorage.setItem(KEY, id); else localStorage.removeItem(KEY); } catch { /* ignore */ }
  }

  selected(): Person | undefined {
    return this.people().find((p) => p.id === this.selectedId());
  }
}
