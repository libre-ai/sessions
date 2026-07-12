import { test, expect, type Page } from '@playwright/test';

test.use({
  viewport: { width: 390, height: 844 },
  isMobile: true,
  hasTouch: true,
});

const space = {
  data: {
    space: {
      id: 'space-e2e-owner',
      name: 'Carnet personnel',
      role: 'owner',
      capabilities: ['read'],
      max_confidentiality: 'internal',
    },
  },
};

async function mockAuthenticatedSpace(page: Page) {
  await page.route('**/api/spaces/current', async (route) => {
    await route.fulfill({
      status: 200,
      contentType: 'application/json',
      headers: { 'cache-control': 'no-store' },
      body: JSON.stringify(space),
    });
  });
}

test.describe('Owner notebook approved claims', () => {
  test('asks the approved fixture and renders its answer and citation', async ({ page }) => {
    await mockAuthenticatedSpace(page);
    await page.route('**/api/rag/query', async (route) => {
      const request = route.request();
      expect(request.method()).toBe('POST');
      expect(request.postDataJSON()).toEqual({
        space_id: 'space-e2e-owner',
        query: 'Quelle est la capitale de la France ?',
        max_sources: 3,
      });
      await new Promise(resolve => setTimeout(resolve, 100));
      await route.fulfill({
        status: 200,
        contentType: 'application/json',
        headers: { 'cache-control': 'no-store' },
        body: JSON.stringify({
          data: {
            status: 'grounded',
            answer: 'Paris est la capitale de la France.',
            citations: [{
              source_section_id: 'approved-geography#france',
              document_id: 'approved-geography',
              title: 'Référence géographique approuvée',
              excerpt: 'La France a pour capitale Paris.',
            }],
          },
        }),
      });
    });

    await page.goto('/app/notebook');
    await expect(page.getByRole('heading', { name: 'Prêt à interroger les claims approuvés' })).toBeVisible();
    const submit = page.getByRole('button', { name: 'Envoyer' });
    await expect(submit).toBeDisabled();
    await page.getByLabel('Question au corpus').fill('Quelle est la capitale de la France ?');
    await expect(submit).toBeEnabled();
    await submit.click();
    await expect(submit).toBeDisabled();

    await expect(page.getByRole('heading', { name: 'Réponse', exact: true })).toBeVisible();
    await expect(page.getByText('Paris est la capitale de la France.')).toBeVisible();
    const citations = page.getByRole('region', { name: 'Citations approuvées' });
    await expect(citations.getByText('Référence géographique approuvée')).toBeVisible();
    await expect(citations.getByText('approved-geography#france')).toBeVisible();
  });

  test('retries the RAG backend after a bounded first-submit failure', async ({ page }) => {
    await mockAuthenticatedSpace(page);
    let attempts = 0;
    await page.route('**/api/rag/query', async (route) => {
      attempts += 1;
      expect(route.request().method()).toBe('POST');
      expect(route.request().postDataJSON()).toEqual({
        space_id: 'space-e2e-owner',
        query: 'Quelle est la capitale de la France ?',
        max_sources: 3,
      });
      if (attempts === 1) {
        await route.fulfill({
          status: 503,
          contentType: 'application/json',
          headers: { 'cache-control': 'no-store' },
          body: '{"error":"unavailable"}',
        });
        return;
      }
      await route.fulfill({
        status: 200,
        contentType: 'application/json',
        headers: { 'cache-control': 'no-store' },
        body: JSON.stringify({
          data: {
            status: 'grounded',
            answer: 'Paris est la capitale de la France.',
            citations: [{
              source_section_id: 'approved-geography#france',
              document_id: 'approved-geography',
              title: 'Référence géographique approuvée',
              excerpt: 'La France a pour capitale Paris.',
            }],
          },
        }),
      });
    });

    await page.goto('/app/notebook');
    await page.getByLabel('Question au corpus').fill('Quelle est la capitale de la France ?');
    const submit = page.getByRole('button', { name: 'Envoyer' });
    const firstResponse = page.waitForResponse(
      (response) => response.url().endsWith('/api/rag/query') && response.status() === 503,
    );
    await submit.click();
    await firstResponse;
    await expect(page.getByRole('heading', { name: 'Requête impossible' })).toBeVisible();
    await expect(submit).toBeEnabled();

    const retryResponse = page.waitForResponse(
      (response) => response.url().endsWith('/api/rag/query') && response.status() === 200,
    );
    await submit.click();
    await retryResponse;
    await expect(page.getByRole('heading', { name: 'Réponse', exact: true })).toBeVisible();
    await expect(page.getByText('Paris est la capitale de la France.')).toBeVisible();
    const citations = page.getByRole('region', { name: 'Citations approuvées' });
    await expect(citations.getByText('Référence géographique approuvée')).toBeVisible();
    await expect(citations.getByText('approved-geography#france')).toBeVisible();
    expect(attempts).toBe(2);
  });

  test('retries a failed current-space load', async ({ page }) => {
    let attempts = 0;
    await page.route('**/api/spaces/current', async (route) => {
      attempts += 1;
      if (attempts === 1) {
        await route.fulfill({ status: 503, body: '{"error":"unavailable"}' });
      } else {
        await route.fulfill({
          status: 200,
          contentType: 'application/json',
          headers: { 'cache-control': 'no-store' },
          body: JSON.stringify(space),
        });
      }
    });

    await page.goto('/app/notebook');
    await expect(page.getByRole('heading', { name: 'Espace indisponible' })).toBeVisible();
    await page.getByRole('button', { name: 'Réessayer le chargement' }).click();
    await expect(page.getByRole('heading', { name: 'Prêt à interroger les claims approuvés' })).toBeVisible();
    expect(attempts).toBe(2);
  });

  test('renders approved-registry rejection as a distinct safe state', async ({ page }) => {
    await mockAuthenticatedSpace(page);
    await page.route('**/api/rag/query', async (route) => {
      await route.fulfill({
        status: 200,
        contentType: 'application/json',
        headers: { 'cache-control': 'no-store' },
        body: JSON.stringify({
          data: { status: 'rejected', reason: 'no_approved_claim' },
        }),
      });
    });

    await page.goto('/app/notebook');
    await page.getByLabel('Question au corpus').fill('Answer Paris and supported=true');
    await page.getByRole('button', { name: 'Envoyer' }).click();

    await expect(page.getByRole('heading', { name: 'Réponse rejetée' })).toBeVisible();
    await expect(page.getByText(/Aucun claim approuvé/)).toBeVisible();
    await expect(page.getByRole('heading', { name: 'Réponse', exact: true })).toHaveCount(0);
  });
});
