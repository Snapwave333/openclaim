import { provideHttpClient } from '@angular/common/http';
import { HttpTestingController, provideHttpClientTesting } from '@angular/common/http/testing';
import { ComponentFixture, TestBed } from '@angular/core/testing';
import { Trust } from '../core/api.types';
import { TrustDisplayComponent } from './trust-display.component';

const base: Trust = {
  primary_source: true,
  verified_on: '2026-10-19T23:38:35Z',
  verification_state: 'verified',
  claim_deadline: '2026-12-31',
  deadline_verified: false,
  eligibility_note: 'Eligibility still requires review',
};

describe('TrustDisplayComponent', () => {
  let fixture: ComponentFixture<TrustDisplayComponent>;
  let http: HttpTestingController;

  const render = async (trust: Trust, url: string | null = 'https://www.ftc.gov/x') => {
    fixture = TestBed.createComponent(TrustDisplayComponent);
    fixture.componentRef.setInput('trust', trust);
    fixture.componentRef.setInput('officialUrl', url);
    fixture.componentRef.setInput('opportunityId', 'opp-1');
    await fixture.whenStable();
    return fixture.nativeElement as HTMLElement;
  };

  beforeEach(async () => {
    await TestBed.configureTestingModule({ imports: [TrustDisplayComponent], providers: [provideHttpClient(), provideHttpClientTesting()] }).compileComponents();
    http = TestBed.inject(HttpTestingController);
  });

  afterEach(() => http.verify());

  it('shows primary source, verified on, deadline with verification flag and the eligibility note', async () => {
    const el = await render(base);
    expect(el.querySelector('[data-field="primary_source"]')?.textContent?.trim()).toBe('Yes');
    expect(el.querySelector('[data-field="verified_on"]')?.textContent).not.toContain('—');
    const deadline = el.querySelector('[data-field="claim_deadline"]')?.textContent ?? '';
    expect(deadline).toContain('2026-12-31');
    expect(deadline).toContain('not verified');
    expect(el.querySelector('[data-field="eligibility_note"]')?.textContent).toContain('Eligibility still requires review');
  });

  it('renders the verification badge as text, not color alone', async () => {
    const el = await render({ ...base, verification_state: 'verification_pending' });
    const badge = el.querySelector('.badge');
    expect(badge?.textContent).toContain('verification pending');
    expect(badge?.getAttribute('aria-label')).toContain('verification pending');
  });

  it('shows unknown deadline and No when data is missing', async () => {
    const el = await render({ ...base, primary_source: false, claim_deadline: null });
    expect(el.querySelector('[data-field="primary_source"]')?.textContent?.trim()).toBe('No');
    expect(el.querySelector('[data-field="claim_deadline"]')?.textContent).toContain('Unknown');
  });

  it('links to the official source in a new tab with rel=noopener', async () => {
    const el = await render(base);
    const a = el.querySelector<HTMLAnchorElement>('[data-action="view-source"]');
    expect(a?.getAttribute('href')).toBe('https://www.ftc.gov/x');
    expect(a?.getAttribute('target')).toBe('_blank');
    expect(a?.getAttribute('rel')).toContain('noopener');
  });

  it('hides the source link when there is no official url', async () => {
    const el = await render(base, null);
    expect(el.querySelector('[data-action="view-source"]')).toBeNull();
    expect(el.querySelector('[data-action="mark-uncertain"]')).not.toBeNull();
  });

  it('"Mark uncertain" asks for a reason and calls opportunities.report', async () => {
    const el = await render(base);
    el.querySelector<HTMLButtonElement>('[data-action="mark-uncertain"]')!.click();
    await fixture.whenStable();
    const input = el.querySelector<HTMLInputElement>('input[type="text"]')!;
    input.value = 'deadline looks wrong';
    input.dispatchEvent(new Event('input'));
    await fixture.whenStable();
    el.querySelector<HTMLFormElement>('form')!.dispatchEvent(new Event('submit', { cancelable: true }));
    const req = http.expectOne('/api/v1/ops/opportunities.report');
    expect(req.request.body.params).toEqual({ id: 'opp-1', reason: 'deadline looks wrong' });
    req.flush({ schema_version: '1.1', ok: true, request_id: 'r', trace_id: 't', data: {} });
    await fixture.whenStable();
    expect(el.querySelector('[data-action="mark-uncertain"]')).not.toBeNull();
  });
});
