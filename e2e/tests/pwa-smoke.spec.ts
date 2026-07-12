import { test, expect, type Page } from '@playwright/test';

async function mockPortableNotebook(page: Page) {
  // API mocks must stay portable even when WebKit lets a newly claimed worker
  // bypass page routing. Worker activation itself is proved by the CSP smoke.
  await page.addInitScript(() => {
    Object.defineProperty(ServiceWorkerContainer.prototype, 'register', {
      configurable: true,
      value: () => Promise.reject(new Error('service worker disabled for isolated API mock')),
    });
  });
  await page.route('**/api/spaces/current', route => route.fulfill({
    status: 200,
    contentType: 'application/json',
    headers: { 'cache-control': 'no-store' },
    body: JSON.stringify({
      data: {
        space: {
          id: 'space-portable-smoke',
          name: 'Carnet portable',
          role: 'owner',
          capabilities: ['read'],
          max_confidentiality: 'internal',
        },
      },
    }),
  }));
  await page.route('**/api/rag/query', async route => {
    expect(route.request().method()).toBe('POST');
    expect(route.request().postDataJSON()).toEqual({
      space_id: 'space-portable-smoke',
      query: 'Quelle preuve portable ?',
      max_sources: 3,
    });
    await route.fulfill({
      status: 200,
      contentType: 'application/json',
      headers: { 'cache-control': 'no-store' },
      body: JSON.stringify({
        data: {
          status: 'grounded',
          answer: 'La preuve portable est isolée par contexte navigateur.',
          citations: [{
            source_section_id: 'portable-smoke#isolation',
            document_id: 'portable-smoke',
            title: 'Preuve portable',
            excerpt: 'Chaque projet reçoit un contexte neuf et des routes mock locales.',
          }],
        },
      }),
    });
  });
}

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

test('runs the isolated critical notebook question to answer and citation flow', async ({ page }) => {
  await mockPortableNotebook(page);
  await page.goto('/app/notebook');
  await expect(page.getByRole('heading', { name: 'Prêt à interroger les claims approuvés' })).toBeVisible();
  await page.getByLabel('Question au corpus').fill('Quelle preuve portable ?');
  await page.getByRole('button', { name: 'Envoyer' }).click();

  await expect(page.getByRole('heading', { name: 'Réponse', exact: true })).toBeVisible();
  await expect(page.getByText('La preuve portable est isolée par contexte navigateur.')).toBeVisible();
  const citations = page.getByRole('region', { name: 'Citations approuvées' });
  await expect(citations.getByText('Preuve portable')).toBeVisible();
  await expect(citations.getByText('portable-smoke#isolation')).toBeVisible();
});

test('keeps /app base-path routes coherent through browser back and forward', async ({ page }) => {
  await page.goto('/app');
  await expect(page).toHaveURL(/\/app\/?$/);
  await expect(page.getByRole('heading', { name: 'Travaillez depuis vos propres sources.' })).toBeVisible();

  await page.getByRole('link', { name: 'Chat RAG' }).first().click();
  await expect(page).toHaveURL(/\/app\/notebook$/);
  await expect(page.getByRole('heading', { name: 'Interroger votre corpus' })).toBeVisible();
  await page.getByRole('link', { name: 'Corpus', exact: true }).click();
  await expect(page).toHaveURL(/\/app\/corpus$/);
  await expect(page.getByRole('heading', { name: 'Corpus', exact: true })).toBeVisible();

  await page.goBack();
  await expect(page).toHaveURL(/\/app\/notebook$/);
  await expect(page.getByRole('heading', { name: 'Interroger votre corpus' })).toBeVisible();
  await page.goForward();
  await expect(page).toHaveURL(/\/app\/corpus$/);
  await expect(page.getByRole('heading', { name: 'Corpus', exact: true })).toBeVisible();
});
