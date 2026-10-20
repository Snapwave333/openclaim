import { provideHttpClient } from '@angular/common/http';
import { HttpTestingController, provideHttpClientTesting } from '@angular/common/http/testing';
import { TestBed } from '@angular/core/testing';
import { ApiService, CSRF_HEADER } from './api.service';
import { ApiError } from './api.types';

const envelope = (over: Record<string, unknown>) => ({ schema_version: '1.1', ok: true, request_id: 'r1', trace_id: 't1', warnings: [], ...over });

describe('ApiService', () => {
  let api: ApiService;
  let http: HttpTestingController;

  beforeEach(() => {
    TestBed.configureTestingModule({ providers: [provideHttpClient(), provideHttpClientTesting()] });
    api = TestBed.inject(ApiService);
    http = TestBed.inject(HttpTestingController);
  });

  afterEach(() => http.verify());

  it('unwraps data from a successful envelope', async () => {
    const p = api.op<{ items: string[] }>('daemon.status', {});
    const req = http.expectOne('/api/v1/ops/daemon.status');
    expect(req.request.method).toBe('POST');
    expect(req.request.body).toEqual({ params: {}, dry_run: false, approval_id: null });
    req.flush(envelope({ data: { items: ['a'] } }));
    await expect(p).resolves.toEqual({ items: ['a'] });
  });

  it('exposes pagination through opFull', async () => {
    const p = api.opFull('opportunities.list', { limit: 1 });
    http.expectOne('/api/v1/ops/opportunities.list').flush(envelope({ data: { items: [] }, pagination: { limit: 1, next_cursor: 'abc', total: 9 } }));
    const r = await p;
    expect(r.envelope.pagination?.next_cursor).toBe('abc');
    expect(r.envelope.pagination?.total).toBe(9);
  });

  it('throws ApiError with code and message when ok=false (non-2xx status)', async () => {
    const p = api.op('people.list');
    http.expectOne('/api/v1/ops/people.list').flush(
      envelope({ ok: false, error: { code: 'NOT_FOUND', message: 'nope', retryable: false } }),
      { status: 404, statusText: 'Not Found' },
    );
    await expect(p).rejects.toMatchObject({ name: 'ApiError', code: 'NOT_FOUND', message: 'nope', status: 404 });
  });

  it('throws ApiError when ok=false even with HTTP 200', async () => {
    const p = api.op('x');
    http.expectOne('/api/v1/ops/x').flush(envelope({ ok: false, error: { code: 'CONFLICT', message: 'changed', retryable: true } }));
    const err = await p.catch((e: unknown) => e);
    expect(err).toBeInstanceOf(ApiError);
    expect((err as ApiError).retryable).toBe(true);
  });

  it('carries approval_id for APPROVAL_REQUIRED', async () => {
    const p = api.op('sources.refresh');
    http.expectOne('/api/v1/ops/sources.refresh').flush(
      envelope({ ok: false, error: { code: 'APPROVAL_REQUIRED', message: 'ask', retryable: false, approval_id: 'ap_1' } }),
      { status: 428, statusText: 'Precondition Required' },
    );
    const err = (await p.catch((e: unknown) => e)) as ApiError;
    expect(err.code).toBe('APPROVAL_REQUIRED');
    expect(err.approvalId).toBe('ap_1');
  });

  it('login stores the csrf token in memory and sends it on later mutations', async () => {
    const login = api.login('secret');
    const lreq = http.expectOne('/api/v1/auth/login');
    expect(lreq.request.body).toEqual({ passphrase: 'secret' });
    expect(lreq.request.headers.has(CSRF_HEADER)).toBe(false);
    lreq.flush(envelope({ data: { csrf: 'occ_token', expires_at: 'later' } }));
    await login;
    expect(api.hasCsrf()).toBe(true);
    expect(localStorage.getItem('csrf')).toBeNull();

    const p = api.op('opportunities.bookmark', { id: '1', on: true });
    const req = http.expectOne('/api/v1/ops/opportunities.bookmark');
    expect(req.request.headers.get(CSRF_HEADER)).toBe('occ_token');
    req.flush(envelope({ data: {} }));
    await p;

    const p2 = api.post('/agents/approvals/a1/decision', { decision: 'approve', digest: 'd' });
    const req2 = http.expectOne('/api/v1/agents/approvals/a1/decision');
    expect(req2.request.headers.get(CSRF_HEADER)).toBe('occ_token');
    req2.flush(envelope({ data: {} }));
    await p2;
  });

  it('GET requests build a query string and skip empty values', async () => {
    const p = api.get('/opportunities', { q: 'walmart', status: '', only_verified: true, limit: 50, cursor: null });
    const req = http.expectOne((r) => r.url.startsWith('/api/v1/opportunities'));
    expect(req.request.method).toBe('GET');
    expect(req.request.urlWithParams).toBe('/api/v1/opportunities?q=walmart&only_verified=true&limit=50');
    req.flush(envelope({ data: { items: [] } }));
    await p;
  });

  it('flags a lost owner session on PERMISSION_DENIED about the session', async () => {
    expect(api.sessionLost()).toBe(false);
    const p = api.op('people.list');
    http.expectOne('/api/v1/ops/people.list').flush(
      envelope({ ok: false, error: { code: 'PERMISSION_DENIED', message: "'people.list' needs an unlocked owner session or an agent token", retryable: false } }),
      { status: 403, statusText: 'Forbidden' },
    );
    await p.catch(() => undefined);
    expect(api.sessionLost()).toBe(true);
  });

  it('does not flag session loss for other permission errors', async () => {
    const p = api.login('bad');
    http.expectOne('/api/v1/auth/login').flush(
      envelope({ ok: false, error: { code: 'PERMISSION_DENIED', message: 'passphrase does not match', retryable: false } }),
      { status: 403, statusText: 'Forbidden' },
    );
    await p.catch(() => undefined);
    expect(api.sessionLost()).toBe(false);
    expect(api.hasCsrf()).toBe(false);
  });

  it('maps a network failure to UNAVAILABLE', async () => {
    const p = api.op('daemon.status');
    http.expectOne('/api/v1/ops/daemon.status').error(new ProgressEvent('error'), { status: 0 });
    const err = (await p.catch((e: unknown) => e)) as ApiError;
    expect(err.code).toBe('UNAVAILABLE');
  });

  it('logout clears the csrf token even if the request fails', async () => {
    const login = api.login('secret');
    http.expectOne('/api/v1/auth/login').flush(envelope({ data: { csrf: 'occ_token', expires_at: 'later' } }));
    await login;
    const p = api.logout();
    http.expectOne('/api/v1/auth/logout').flush('boom', { status: 500, statusText: 'Server Error' });
    await p.catch(() => undefined);
    expect(api.hasCsrf()).toBe(false);
  });
});
