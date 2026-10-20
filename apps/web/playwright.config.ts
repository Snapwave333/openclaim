import { defineConfig, devices } from '@playwright/test';

/**
 * E2E against a running, seeded daemon on 127.0.0.1:7865 (see README).
 * By default `ng serve` (port 4200, proxying /api to the daemon) is started automatically.
 * Set BASE_URL=http://127.0.0.1:7865 to test the production build served by the daemon itself.
 */
const baseURL = process.env['BASE_URL'] ?? 'http://localhost:4200';
const useDevServer = !process.env['BASE_URL'];

export default defineConfig({
  testDir: './e2e',
  timeout: 60_000,
  expect: { timeout: 10_000 },
  fullyParallel: false,
  workers: 1,
  retries: 0,
  reporter: [['list'], ['html', { open: 'never', outputFolder: 'e2e-report' }]],
  outputDir: 'e2e-results',
  use: {
    baseURL,
    trace: 'retain-on-failure',
    screenshot: 'only-on-failure',
  },
  projects: [{ name: 'chromium', use: { ...devices['Desktop Chrome'] } }],
  webServer: useDevServer
    ? { command: 'npx ng serve --port 4200', url: 'http://localhost:4200', reuseExistingServer: true, timeout: 120_000 }
    : undefined,
});
