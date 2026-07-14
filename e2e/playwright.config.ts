import { defineConfig, devices } from '@playwright/test';

const baseURL = process.env.TEST_BASE_URL || 'http://localhost:3000';
const externalTarget = Boolean(process.env.TEST_BASE_URL);

export default defineConfig({
  testDir: './tests',
  fullyParallel: true,
  forbidOnly: !!process.env.CI,
  retries: process.env.CI ? 2 : 0,
  workers: process.env.CI ? 1 : undefined,
  reporter: 'html',
  use: {
    baseURL,
    trace: 'on-first-retry',
  },
  projects: [
    {
      name: 'chromium',
      testIgnore: /guest\.spec\.ts/,
      use: { ...devices['Desktop Chrome'] },
    },
    {
      name: 'firefox-smoke',
      testMatch: /pwa-smoke\.spec\.ts/,
      use: { ...devices['Desktop Firefox'] },
    },
    {
      name: 'webkit-smoke',
      testMatch: /pwa-smoke\.spec\.ts/,
      use: { ...devices['Desktop Safari'] },
    },
    {
      name: 'mobile-chromium-smoke',
      testMatch: /pwa-smoke\.spec\.ts/,
      use: { ...devices['Pixel 7'] },
    },
    {
      name: 'guest-mobile-chromium',
      testMatch: /guest\.spec\.ts/,
      use: { ...devices['Pixel 7'], trace: 'off', video: 'off', screenshot: 'off' },
    },
    {
      name: 'guest-mobile-webkit',
      testMatch: /guest\.spec\.ts/,
      use: { ...devices['iPhone 13'], trace: 'off', video: 'off', screenshot: 'off' },
    },
  ],
  webServer: externalTarget ? undefined : {
    command: 'cd .. && ./scripts/verify-owner-app.sh && ./scripts/verify-join-app.sh && INGEST_TOKEN=playwright-explicit-non-secret-000000000000 PORT=3000 cargo run --release --bin presto-server',
    url: 'http://localhost:3000/health',
    reuseExistingServer: !process.env.CI && process.env.KEYCLOAK_E2E !== '1',
    timeout: 120000,
  },
});
