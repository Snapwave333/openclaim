import AxeBuilder from '@axe-core/playwright';
import { BrowserContext, Page, expect, test } from '@playwright/test';
import { login } from './helpers';

test.describe.configure({ mode: 'serial' });

/** Accessibility smoke: WCAG 2.x A/AA rules via axe-core on three pages (login, discover, overview). */
async function audit(page: Page, name: string): Promise<void> {
  const results = await new AxeBuilder({ page }).withTags(['wcag2a', 'wcag2aa', 'wcag21aa']).analyze();
  const summary = results.violations.map((v) => ({ id: v.id, impact: v.impact, help: v.help, nodes: v.nodes.map((n) => n.target) }));
  expect(summary, `${name}: ${JSON.stringify(summary, null, 2)}`).toEqual([]);
}

test('axe: login page', async ({ page }) => {
  await page.goto('/login');
  await expect(page.getByRole('heading', { name: 'Unlock OpenClaim' })).toBeVisible();
  await audit(page, 'login');
});

test.describe('axe: unlocked pages', () => {
  let context: BrowserContext;
  let page: Page;
  test.beforeAll(async ({ browser }) => { context = await browser.newContext(); page = await context.newPage(); await login(page); });
  test.afterAll(async () => { await context.close(); });

  test('axe: overview', async () => {
    await expect(page.getByRole('heading', { name: 'Overview', level: 1 })).toBeVisible();
    await expect(page.getByRole('row', { name: /FTC Refund Programs/ })).toBeVisible();
    await audit(page, 'overview');
  });

  test('axe: discover (dark and light)', async () => {
    await page.getByRole('link', { name: 'Discover' }).click();
    await expect(page.locator('[data-opportunity-id]').first()).toBeVisible();
    for (const scheme of ['dark', 'light'] as const) {
      // The GUI follows prefers-color-scheme; reduced motion disables the theme-change transitions so axe sees settled colors.
      await page.emulateMedia({ colorScheme: scheme, reducedMotion: 'reduce' });
      await expect(page.locator('html')).toHaveAttribute('data-theme', scheme);
      await audit(page, `discover/${scheme}`);
    }
  });
});
