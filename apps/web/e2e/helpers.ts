import { Page, expect } from '@playwright/test';

/** Test passphrase used by the seed in README (dev daemon only). */
export const PASSPHRASE = process.env['OPENCLAIM_E2E_PASSPHRASE'] ?? 'correct-horse';

/**
 * Unlock the GUI. The daemon allows 5 unlock attempts per minute, so each spec file logs in once
 * (shared page) and this helper waits out the window if the limit was hit by a previous run.
 */
export async function login(page: Page): Promise<void> {
  await page.goto('/login');
  await expect(page.getByRole('heading', { name: 'Unlock OpenClaim' })).toBeVisible();
  for (let attempt = 0; attempt < 3; attempt++) {
    await page.getByLabel('Passphrase').fill(PASSPHRASE);
    await page.getByRole('button', { name: 'Unlock' }).click();
    const nav = page.getByRole('navigation', { name: 'Main' });
    const alert = page.getByRole('alert');
    await Promise.race([nav.waitFor({ state: 'visible', timeout: 15_000 }), alert.waitFor({ state: 'visible', timeout: 15_000 })]);
    if (await nav.isVisible()) return;
    const text = await alert.textContent();
    if (!/too many unlock attempts/i.test(text ?? '')) throw new Error(`login failed: ${text}`);
    await page.waitForTimeout(61_000);
  }
  throw new Error('login failed after waiting for the unlock rate limit');
}
