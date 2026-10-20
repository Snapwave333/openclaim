import { BrowserContext, Page, expect, test } from '@playwright/test';
import { login } from './helpers';

test.describe.configure({ mode: 'serial' });

test.describe('screens render against the seeded daemon', () => {
  let context: BrowserContext;
  let page: Page;
  test.beforeAll(async ({ browser }) => { context = await browser.newContext(); page = await context.newPage(); await login(page); });
  test.afterAll(async () => { await context.close(); });

  test('overview shows tiles and source health with coverage-gap wording', async () => {
    await expect(page.getByRole('heading', { name: 'Overview', level: 1 })).toBeVisible();
    await expect(page.locator('[data-tile="potential"]')).toBeVisible();
    await expect(page.getByText(/coverage gap/i).first()).toBeVisible();
    await expect(page.getByRole('row', { name: /FTC Refund Programs/ })).toBeVisible();
  });

  test('my matches lists the seeded person and explains a match', async () => {
    await page.getByRole('link', { name: 'My Matches' }).click();
    await expect(page.getByLabel('Person', { exact: true })).toContainText('Ann Example');
    await page.getByRole('button', { name: 'Re-evaluate all' }).click();
    const row = page.locator('[data-opportunity-id]').filter({ hasText: 'Walmart Refunds' }).first();
    await expect(row).toBeVisible();
    await row.getByRole('button', { name: 'Why?' }).click();
    await expect(row.locator('[data-explain]')).toContainText('Assessment');
    await expect(row.getByRole('button', { name: 'I am not part of this class' })).toBeVisible();
  });

  test('sources page lists connectors and explains blocked = coverage gap', async () => {
    await page.getByRole('link', { name: 'Sources' }).click();
    await expect(page.locator('[data-source-id="ftc-refunds"]')).toBeVisible();
    await expect(page.getByText(/Blocked means coverage gap/)).toBeVisible();
    await expect(page.getByLabel('Page HTML')).toBeVisible();
  });

  test('agent activity shows durable jobs with progress bars', async () => {
    await page.getByRole('link', { name: 'Agent Activity' }).click();
    await expect(page.getByRole('heading', { name: 'Agent Activity', level: 1 })).toBeVisible();
    await expect(page.locator('[data-job-id]').first()).toBeVisible();
    await expect(page.getByRole('progressbar').first()).toBeVisible();
  });

  test('approvals, claims, vault and property pages load', async () => {
    for (const name of ['Approvals', 'Claims', 'Evidence Vault', 'Property Search'] as const) {
      await page.getByRole('link', { name }).click();
      await expect(page.getByRole('heading', { name, level: 1 })).toBeVisible();
    }
    await expect(page.getByRole('row', { name: /missingmoney|unclaimed|Alberta/i }).first()).toBeVisible();
  });

  test('settings has the no-AI statement and a type-to-confirm wipe', async () => {
    await page.getByRole('link', { name: 'Settings' }).click();
    await expect(page.getByText('No AI inside.')).toBeVisible();
    await page.getByRole('button', { name: 'Wipe…' }).click();
    const dialog = page.getByRole('dialog');
    await expect(dialog).toBeVisible();
    await expect(dialog.locator('[data-action="confirm"]')).toBeDisabled();
    await page.keyboard.press('Escape');
    await expect(dialog).toHaveCount(0);
  });

  test('theme toggle persists in localStorage and the csrf token does not', async () => {
    await page.getByLabel('Theme').selectOption('light');
    await expect(page.locator('html')).toHaveAttribute('data-theme', 'light');
    const prefs = await page.evaluate(() => localStorage.getItem('openclaim.prefs'));
    expect(JSON.parse(prefs ?? '{}')).toMatchObject({ theme: 'light' });
    const everything = await page.evaluate(() => JSON.stringify(localStorage) + JSON.stringify(sessionStorage));
    expect(everything).not.toMatch(/occ_/);
    await page.getByLabel('Theme').selectOption('dark');
    await expect(page.locator('html')).toHaveAttribute('data-theme', 'dark');
  });
});
