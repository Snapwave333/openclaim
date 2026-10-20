import { HttpClient, HttpErrorResponse, HttpHeaders } from '@angular/common/http';
import { Injectable, inject, signal } from '@angular/core';
import { firstValueFrom } from 'rxjs';
import { ApiError, ApiErrorBody, Envelope, OwnerStatus } from './api.types';

export const API_BASE = '/api/v1';
export const CSRF_HEADER = 'X-OpenClaim-CSRF';

export interface OpResult<T> {
  data: T;
  envelope: Envelope<T>;
}

/**
 * Thin client over the daemon's localhost REST API.
 *
 * - Every response is an envelope; `ok=false` becomes a thrown `ApiError`.
 * - The CSRF token lives only in memory (never localStorage) and is sent on every mutation.
 * - A PERMISSION_DENIED about the owner session flips `locked` so the shell can show the login screen.
 */
@Injectable({ providedIn: 'root' })
export class ApiService {
  private readonly http = inject(HttpClient);
  private csrf: string | null = null;

  /** Set when the daemon reports that the owner session is missing or expired. */
  readonly sessionLost = signal(false);

  setCsrf(token: string | null): void { this.csrf = token; }
  hasCsrf(): boolean { return !!this.csrf; }

  private mutationHeaders(): HttpHeaders {
    let h = new HttpHeaders({ 'Content-Type': 'application/json' });
    if (this.csrf) h = h.set(CSRF_HEADER, this.csrf);
    return h;
  }

  /** Generic operation call: `POST /ops/{operation}`. */
  async op<T = unknown>(operation: string, params: Record<string, unknown> = {}, opts: { dryRun?: boolean; approvalId?: string | null } = {}): Promise<T> {
    const r = await this.opFull<T>(operation, params, opts);
    return r.data;
  }

  async opFull<T = unknown>(operation: string, params: Record<string, unknown> = {}, opts: { dryRun?: boolean; approvalId?: string | null } = {}): Promise<OpResult<T>> {
    const body = { params, dry_run: opts.dryRun ?? false, approval_id: opts.approvalId ?? null };
    const env = await this.request<T>(() => this.http.post<Envelope<T>>(`${API_BASE}/ops/${operation}`, body, { headers: this.mutationHeaders() }));
    return { data: env.data as T, envelope: env };
  }

  async get<T = unknown>(path: string, query: Record<string, string | number | boolean | undefined | null> = {}): Promise<OpResult<T>> {
    const qs = Object.entries(query)
      .filter(([, v]) => v !== undefined && v !== null && v !== '')
      .map(([k, v]) => `${encodeURIComponent(k)}=${encodeURIComponent(String(v))}`)
      .join('&');
    const url = `${API_BASE}${path}${qs ? `?${qs}` : ''}`;
    const env = await this.request<T>(() => this.http.get<Envelope<T>>(url));
    return { data: env.data as T, envelope: env };
  }

  async post<T = unknown>(path: string, body: unknown): Promise<OpResult<T>> {
    const env = await this.request<T>(() => this.http.post<Envelope<T>>(`${API_BASE}${path}`, body, { headers: this.mutationHeaders() }));
    return { data: env.data as T, envelope: env };
  }

  // ---- auth -------------------------------------------------------------

  async me(): Promise<OwnerStatus> {
    const r = await this.get<OwnerStatus>('/auth/me');
    return r.data;
  }

  /** After a page reload the cookie may still be valid; ask the daemon for a fresh CSRF token. */
  async refreshCsrf(): Promise<boolean> {
    try {
      const r = await this.get<{ csrf: string }>('/auth/csrf');
      this.csrf = r.data?.csrf ?? null;
      return !!this.csrf;
    } catch {
      return false;
    }
  }

  async login(passphrase: string): Promise<void> {
    const env = await this.request<{ csrf: string; expires_at: string }>(() =>
      this.http.post<Envelope<{ csrf: string; expires_at: string }>>(`${API_BASE}/auth/login`, { passphrase }, { headers: new HttpHeaders({ 'Content-Type': 'application/json' }) }),
    );
    this.csrf = env.data?.csrf ?? null;
    this.sessionLost.set(false);
  }

  async logout(): Promise<void> {
    try {
      await this.request(() => this.http.post<Envelope>(`${API_BASE}/auth/logout`, {}, { headers: this.mutationHeaders() }));
    } finally {
      this.csrf = null;
    }
  }

  // ---- internals --------------------------------------------------------

  private async request<T>(call: () => import('rxjs').Observable<Envelope<T>>): Promise<Envelope<T>> {
    let env: Envelope<T>;
    try {
      env = await firstValueFrom(call());
    } catch (e) {
      throw this.fromHttpError(e);
    }
    if (!env || typeof env !== 'object' || !('ok' in env)) {
      throw new ApiError('INTERNAL', 'malformed response from daemon', 500);
    }
    if (!env.ok) {
      const err = env.error ?? { code: 'INTERNAL', message: 'unknown error', retryable: false };
      this.noteSessionState(err);
      throw new ApiError(err.code, err.message, 200, err, env.request_id);
    }
    return env;
  }

  private fromHttpError(e: unknown): ApiError {
    if (e instanceof ApiError) return e;
    if (e instanceof HttpErrorResponse) {
      const body = e.error as Partial<Envelope> | string | null;
      if (body && typeof body === 'object' && body.error) {
        const err = body.error as ApiErrorBody;
        this.noteSessionState(err);
        return new ApiError(err.code, err.message, e.status, err, body.request_id);
      }
      if (e.status === 0) return new ApiError('UNAVAILABLE', 'The OpenClaim daemon is not reachable on localhost.', 0);
      const text = typeof body === 'string' && body ? body : e.statusText || 'request failed';
      return new ApiError(e.status === 403 ? 'PERMISSION_DENIED' : 'INTERNAL', text, e.status);
    }
    return new ApiError('INTERNAL', e instanceof Error ? e.message : String(e), 0);
  }

  private noteSessionState(err: ApiErrorBody): void {
    if (err.code === 'PERMISSION_DENIED' && /owner session|csrf/i.test(err.message)) {
      this.sessionLost.set(true);
    }
  }
}
