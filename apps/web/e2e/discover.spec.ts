import { BrowserContext, Page, expect, test } from '@playwright/test';
import { login } from './helpers';

test.describe.configure({ mode: 'serial' });

test.describe('login → discover → detail → bookmark', () => {
  let context: BrowserContext;
  let page: Page;
  const consoleErrors: string[] = [];
  test.beforeAll(async ({ browser }) => {
    context = await browser.newContext();
    page = await context.newPage();
    page.on('console', (m) => { if (m.type() === 'error') consoleErrors.push(m.text()); });
    page.on('pageerror', (e) => consoleErrors.push(String(e)));
    await login(page);
  });
  test.afterAll(async () => { await context.close(); });

  test('discover shows the seeded Dr. Squatch record with a verified badge and full trust display', async () => {
    await page.getByRole('link', { name: 'Discover' }).click();
    await expect(page.getByRole('heading', { name: 'Discover', level: 1 })).toBeVisible();
    await page.locator('#q').fill('Squatch');
    await page.getByRole('button', { name: 'Search' }).click();

    const card = page.locator('[data-opportunity-id]').filter({ hasText: 'Dr. Squatch - All Natural Claims' }).first();
    await expect(card).toBeVisible();
    await expect(card.locator('.badge').filter({ hasText: 'verified' }).first()).toBeVisible();
    await expect(card.locator('[data-field="primary_source"]')).toHaveText(/Yes/);
    await expect(card.locator('[data-field="verified_on"]')).not.toHaveText('—');
    await expect(card.locator('[data-field="claim_deadline"]')).toContainText(/Unknown|verified/);
    await expect(card.locator('[data-field="eligibility_note"]')).toContainText('Eligibility still requires review');
    const source = card.locator('[data-action="view-source"]');
    await expect(source).toHaveAttribute('href', /personalcareproductssettlement\.com/);
    await expect(source).toHaveAttribute('target', '_blank');
    await expect(source).toHaveAttribute('rel', /noopener/);
    await expect(card.locator('[data-action="mark-uncertain"]')).toBeVisible();
  });

  test('bookmark toggles through opportunities.bookmark and is reflected after reload', async () => {
    const card = page.locator('[data-opportunity-id]').filter({ hasText: 'Dr. Squatch - All Natural Claims' }).first();
    const bookmark = card.locator('[data-action="bookmark"]');
    const wasPressed = (await bookmark.getAttribute('aria-pressed')) === 'true';
    await bookmark.click();
    await expect(bookmark).toHaveAttribute('aria-pressed', String(!wasPressed));
    // The daemon persisted it: re-run the search and check the state survived
    await page.getByRole('button', { name: 'Search' }).click();
    const again = page.locator('[data-opportunity-id]').filter({ hasText: 'Dr. Squatch - All Natural Claims' }).first().locator('[data-action="bookmark"]');
    await expect(again).toHaveAttribute('aria-pressed', String(!wasPressed));
    // Put it back so the seed stays stable
    await again.click();
    await expect(again).toHaveAttribute('aria-pressed', String(wasPressed));
  });

  test('detail page shows provenance excerpts in the untrusted-text style and a match explanation', async () => {
    const card = page.locator('[data-opportunity-id]').filter({ hasText: 'Dr. Squatch - All Natural Claims' }).first();
    await card.getByRole('link', { name: 'Dr. Squatch - All Natural Claims' }).click();
    await expect(page.getByRole('heading', { name: 'Dr. Squatch - All Natural Claims', level: 1 })).toBeVisible();
    await expect(page.locator('.badge').filter({ hasText: 'verified' }).first()).toBeVisible();

    await page.getByRole('tab', { name: 'Supporting sources' }).click();
    const evidence = page.locator('[data-evidence]');
    await expect(evidence.first()).toBeVisible();
    const untrusted = evidence.first().locator('[data-trust="untrusted_source_text"]');
    await expect(untrusted).toBeVisible();
    await expect(untrusted).toContainText('Untrusted source text');
    const font = await untrusted.evaluate((el) => getComputedStyle(el).fontFamily);
    expect(font.toLowerCase()).toMatch(/mono|consolas|menlo|courier/);
    const borderLeft = await untrusted.evaluate((el) => getComputedStyle(el).borderLeftWidth);
    expect(parseFloat(borderLeft)).toBeGreaterThanOrEqual(3);
    expect(await untrusted.locator('a, img, script').count()).toBe(0);

    await page.getByRole('tab', { name: 'Eligibility' }).click();
    await expect(page.locator('[data-explain]')).toContainText(/manual review|potential match|needs more information|unlikely|not applicable/);
    await expect(page.locator('[data-explain]')).toContainText('Only the official administrator decides eligibility');
  });

  test('no CSP violations or page errors were logged (matters when the daemon serves the build)', async () => {
    const bad = consoleErrors.filter((e) => /Content Security Policy|Refused to|Uncaught|NG0/i.test(e));
    expect(bad, bad.join('; ')).toEqual([]);
  });
});

test.describe('login guards', () => {
  test('protected routes redirect to login when not unlocked', async ({ page }) => {
    await page.goto('/claims');
    await expect(page).toHaveURL(/\/login/);
    await expect(page.getByRole('heading', { name: 'Unlock OpenClaim' })).toBeVisible();
  });

  test('rejects a wrong passphrase and stays on login', async ({ page }) => {
    await page.goto('/login');
    await page.getByLabel('Passphrase').fill('definitely-wrong');
    await page.getByRole('button', { name: 'Unlock' }).click();
    await expect(page.getByRole('alert')).toContainText(/passphrase|unlock attempts/i);
    await expect(page.getByRole('navigation', { name: 'Main' })).toHaveCount(0);
  });
});
