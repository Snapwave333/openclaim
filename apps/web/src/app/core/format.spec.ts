import { daysUntil, humanize, money } from './format';

describe('format', () => {
  it('renders integer minor units as currency', () => {
    expect(money(25000, 'USD')).toBe('$250.00');
    expect(money(5, 'USD')).toBe('$0.05');
    expect(money(null)).toBe('—');
  });
  it('humanizes snake_case', () => {
    expect(humanize('verification_pending')).toBe('verification pending');
    expect(humanize(null)).toBe('—');
  });
  it('computes days until a date', () => {
    expect(daysUntil(null)).toBeNull();
    expect(daysUntil('not a date')).toBeNull();
    expect(daysUntil(new Date(Date.now() + 3 * 86400000).toISOString())).toBeGreaterThanOrEqual(2);
  });
});
