import { test, expect } from '@playwright/test';

test('loads the WASM shell on a deep route with strict CSP and an active worker', async ({ page }) => {
  const violations: string[] = [];
  const consoleErrors: string[] = [];
  page.on('console', message => {
    if (message.type() === 'error') consoleErrors.push(message.text());
  });
  await page.addInitScript(() => {
    document.addEventListener('securitypolicyviolation', event => {
      (globalThis as typeof globalThis & { __cspViolations?: string[] }).__cspViolations ??= [];
      (globalThis as typeof globalThis & { __cspViolations: string[] }).__cspViolations.push(
        `${event.violatedDirective}:${event.blockedURI}`,
      );
    });
  });

  const response = await page.goto('/app/notebook');
  expect(response?.headers()['content-security-policy']).toBe(
    "default-src 'none'; base-uri 'none'; object-src 'none'; frame-ancestors 'none'; form-action 'self'; script-src 'self' 'wasm-unsafe-eval'; style-src 'self'; style-src-attr 'none'; img-src 'self'; font-src 'self'; connect-src 'self'; manifest-src 'self'; worker-src 'self'",
  );
  await expect(page.getByRole('heading', { name: 'Interroger votre corpus' })).toBeVisible();
  await page.evaluate(async () => {
    const registration = await navigator.serviceWorker.ready;
    if (!registration.active) throw new Error('service worker is not active');
  });
  violations.push(...await page.evaluate(() =>
    (globalThis as typeof globalThis & { __cspViolations?: string[] }).__cspViolations ?? [],
  ));
  expect(violations).toEqual([]);
  expect(consoleErrors.filter(message => !message.includes('Failed to load resource'))).toEqual([]);
});
