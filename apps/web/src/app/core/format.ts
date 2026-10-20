/** Formatting helpers. Money is always integer minor units (MDR §8). */

export function money(minor: number | null | undefined, currency = 'USD'): string {
  if (minor === null || minor === undefined || Number.isNaN(minor)) return '—';
  try {
    return new Intl.NumberFormat(undefined, { style: 'currency', currency }).format(minor / 100);
  } catch {
    return `${(minor / 100).toFixed(2)} ${currency}`;
  }
}

export function date(iso: string | null | undefined): string {
  if (!iso) return '—';
  const d = new Date(iso);
  if (Number.isNaN(d.getTime())) return iso;
  // Date-only values (YYYY-MM-DD) are shown as-is to avoid timezone shifts.
  if (/^\d{4}-\d{2}-\d{2}$/.test(iso)) return iso;
  return d.toLocaleString(undefined, { dateStyle: 'medium', timeStyle: 'short' });
}

export function dateOnly(iso: string | null | undefined): string {
  if (!iso) return '—';
  if (/^\d{4}-\d{2}-\d{2}$/.test(iso)) return iso;
  const d = new Date(iso);
  return Number.isNaN(d.getTime()) ? iso : d.toLocaleDateString(undefined, { dateStyle: 'medium' });
}

export function relative(iso: string | null | undefined): string {
  if (!iso) return '—';
  const d = new Date(iso).getTime();
  if (Number.isNaN(d)) return iso;
  const diff = d - Date.now();
  const abs = Math.abs(diff);
  const units: Array<[number, Intl.RelativeTimeFormatUnit]> = [[86400000 * 30, 'month'], [86400000, 'day'], [3600000, 'hour'], [60000, 'minute']];
  for (const [ms, unit] of units) {
    if (abs >= ms) return new Intl.RelativeTimeFormat(undefined, { numeric: 'auto' }).format(Math.round(diff / ms), unit);
  }
  return 'just now';
}

export function humanize(s: string | null | undefined): string {
  if (!s) return '—';
  return s.replace(/_/g, ' ');
}

export function daysUntil(iso: string | null | undefined): number | null {
  if (!iso) return null;
  const d = new Date(iso).getTime();
  if (Number.isNaN(d)) return null;
  return Math.ceil((d - Date.now()) / 86400000);
}
