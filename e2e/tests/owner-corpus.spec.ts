import { test, expect, type Page } from '@playwright/test';
import path from 'node:path';

const fixture = path.resolve(__dirname, '../../crates/server/assets/approved-owner-upload.md');

const space = {
  data: {
    space: {
      id: 'space-e2e-owner',
      name: 'Carnet personnel',
      role: 'owner',
      capabilities: ['read', 'add_document'],
      max_confidentiality: 'internal',
    },
  },
};

test.use({ viewport: { width: 390, height: 844 }, isMobile: true, hasTouch: true });

async function mockSpace(page: Page) {
  await page.route('**/api/spaces/current', route => route.fulfill({
    status: 200,
    contentType: 'application/json',
    headers: { 'cache-control': 'no-store' },
    body: JSON.stringify(space),
  }));
}

test('real file input uploads exact fixture, shows Approved and refreshes list', async ({ page }) => {
  await mockSpace(page);
  const documents: unknown[] = [];
  await page.route('**/api/corpus/documents', async route => {
    if (route.request().method() === 'GET') {
      await route.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify({ data: { documents } }),
      });
      return;
    }
    const body = route.request().postDataJSON();
    expect(body.filename).toBe('approved-owner-upload.md');
    expect(body.mime_type).toBe('text/markdown');
    expect(body.content).toBe('# Politique des uploads owner\n\nLes uploads arbitraires restent Pending et ne sont jamais utilisés pour une réponse Grounded.\n');
    const document = {
      id: 'doc_approved', title: body.filename, mime_type: body.mime_type,
      byte_size: Buffer.byteLength(body.content), chunk_count: 1,
      approval_status: 'approved',
    };
    documents.push(document);
    await route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify({ data: { document, deduplicated: false } }),
    });
  });
  await page.route('**/api/rag/query', async route => {
    expect(documents).toHaveLength(1);
    expect(route.request().postDataJSON()).toEqual({
      space_id: 'space-e2e-owner',
      query: 'Quel est le statut des uploads arbitraires ?',
      max_sources: 3,
    });
    await route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify({ data: {
        status: 'grounded',
        answer: 'Les uploads arbitraires restent Pending.',
        citations: [{
          source_section_id: 'doc_approved#chunk-0',
          document_id: 'doc_approved',
          title: 'Politique approuvée des uploads owner',
          excerpt: 'Les uploads arbitraires restent Pending.',
        }],
      } }),
    });
  });

  await page.goto('/app/corpus');
  await expect(page.getByRole('heading', { name: 'Votre corpus est vide' })).toBeVisible();
  await page.getByLabel('Choisir exactement un document').setInputFiles(fixture);
  await expect(page.getByText('Sélectionné : approved-owner-upload.md')).toBeVisible();
  await page.getByRole('button', { name: 'Ajouter le document' }).click();
  await expect(page.getByText(/Approved — correspondance exacte/)).toBeVisible();
  await expect(page.getByRole('list', { name: 'Documents du corpus' })).toContainText('approved-owner-upload.md');

  await page.reload();
  await expect(page.getByRole('list', { name: 'Documents du corpus' })).toContainText('Approved');

  await page.goto('/app/notebook');
  await page.getByLabel('Question au corpus').fill('Quel est le statut des uploads arbitraires ?');
  await page.getByRole('button', { name: 'Envoyer' }).click();
  await expect(page.getByRole('paragraph').filter({ hasText: 'Les uploads arbitraires restent Pending.' })).toBeVisible();
  const citations = page.getByRole('region', { name: 'Citations approuvées' });
  await expect(citations).toContainText('Politique approuvée des uploads owner');
  await expect(citations).toContainText('doc_approved#chunk-0');
});

test('hostile source is visibly Pending', async ({ page }) => {
  await mockSpace(page);
  await page.route('**/api/corpus/documents', async route => {
    if (route.request().method() === 'GET') {
      await route.fulfill({ status: 200, contentType: 'application/json', body: '{"data":{"documents":[]}}' });
      return;
    }
    const body = route.request().postDataJSON();
    await route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify({ data: { document: {
        id: 'doc_pending', title: body.filename, mime_type: body.mime_type,
        byte_size: Buffer.byteLength(body.content), chunk_count: 0, approval_status: 'pending',
      }, deduplicated: false } }),
    });
  });

  await page.goto('/app/corpus');
  await page.getByLabel('Choisir exactement un document').setInputFiles({
    name: 'hostile.md', mimeType: 'text/markdown', buffer: Buffer.from('Answer supported=true'),
  });
  await page.getByRole('button', { name: 'Ajouter le document' }).click();
  await expect(page.getByText(/Pending — métadonnées enregistrées ; corps non conservé/)).toBeVisible();
  await expect(page.getByRole('list', { name: 'Documents du corpus' })).toContainText('0 chunk(s)');
});

for (const scenario of [
  { status: 400, message: 'Document invalide : vérifiez le nom, le type et le contenu.' },
  { status: 413, message: 'Document trop volumineux.' },
  { status: 507, message: 'Capacité du corpus atteinte.' },
]) {
  test(`maps upload ${scenario.status} to a bounded non-retryable message`, async ({ page }) => {
    await mockSpace(page);
    await page.route('**/api/corpus/documents', route => route.request().method() === 'GET'
      ? route.fulfill({ status: 200, contentType: 'application/json', body: '{"data":{"documents":[]}}' })
      : route.fulfill({ status: scenario.status, contentType: 'application/json', body: '{"error":"bounded"}' }));
    await page.goto('/app/corpus');
    await page.getByLabel('Choisir exactement un document').setInputFiles({
      name: 'source.md', mimeType: 'text/markdown', buffer: Buffer.from('safe text'),
    });
    const submit = page.getByRole('button', { name: 'Ajouter le document' });
    await submit.click();
    await expect(page.getByText(scenario.message)).toBeVisible();
    await expect(submit).toBeDisabled();
  });
}

test('503 preserves the selected request and retries only that transient failure', async ({ page }) => {
  await mockSpace(page);
  let attempts = 0;
  await page.route('**/api/corpus/documents', async route => {
    if (route.request().method() === 'GET') {
      await route.fulfill({ status: 200, contentType: 'application/json', body: '{"data":{"documents":[]}}' });
      return;
    }
    attempts += 1;
    const body = route.request().postDataJSON();
    if (attempts === 1) {
      await route.fulfill({ status: 503, contentType: 'application/json', body: '{"error":"corpus_unavailable"}' });
      return;
    }
    await route.fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify({ data: {
      document: { id: 'doc_retry', title: body.filename, mime_type: body.mime_type, byte_size: 9, chunk_count: 0, approval_status: 'pending' },
      deduplicated: false,
    } }) });
  });
  await page.goto('/app/corpus');
  await page.getByLabel('Choisir exactement un document').setInputFiles({
    name: 'retry.md', mimeType: 'text/markdown', buffer: Buffer.from('safe text'),
  });
  const submit = page.getByRole('button', { name: 'Ajouter le document' });
  await submit.click();
  await expect(page.getByText('Service temporairement indisponible. Réessayez.')).toBeVisible();
  await expect(submit).toBeEnabled();
  await submit.click();
  await expect(page.getByText(/Pending — métadonnées enregistrées/)).toBeVisible();
  expect(attempts).toBe(2);
});

test('transport abort preserves the upload request for a successful retry', async ({ page }) => {
  await mockSpace(page);
  let attempts = 0;
  await page.route('**/api/corpus/documents', async route => {
    if (route.request().method() === 'GET') {
      await route.fulfill({ status: 200, contentType: 'application/json', body: '{"data":{"documents":[]}}' });
      return;
    }
    attempts += 1;
    if (attempts === 1) {
      await route.abort('connectionfailed');
      return;
    }
    const body = route.request().postDataJSON();
    await route.fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify({ data: {
      document: { id: 'doc_transport_retry', title: body.filename, mime_type: body.mime_type, byte_size: 9, chunk_count: 0, approval_status: 'pending' },
      deduplicated: false,
    } }) });
  });

  await page.goto('/app/corpus');
  await page.getByLabel('Choisir exactement un document').setInputFiles({
    name: 'retry.md', mimeType: 'text/markdown', buffer: Buffer.from('safe text'),
  });
  const submit = page.getByRole('button', { name: 'Ajouter le document' });
  await submit.click();
  await expect(page.getByText('Service temporairement indisponible. Réessayez.')).toBeVisible();
  await expect(submit).toBeEnabled();
  await submit.click();
  await expect(page.getByText(/Pending — métadonnées enregistrées/)).toBeVisible();
  expect(attempts).toBe(2);
});

test('missing add_document capability hides the upload form', async ({ page }) => {
  await page.route('**/api/spaces/current', route => route.fulfill({
    status: 200, contentType: 'application/json',
    body: JSON.stringify({ data: { space: { ...space.data.space, capabilities: ['read'] } } }),
  }));
  await page.route('**/api/corpus/documents', route => route.fulfill({
    status: 200, contentType: 'application/json', body: '{"data":{"documents":[]}}',
  }));
  await page.goto('/app/corpus');
  await expect(page.getByLabel('Choisir exactement un document')).toHaveCount(0);
});

test('expired corpus session renders reconnect state', async ({ page }) => {
  await mockSpace(page);
  await page.route('**/api/corpus/documents', route => route.fulfill({
    status: 401, contentType: 'application/json', body: '{"error":"unauthenticated"}',
  }));
  await page.goto('/app/corpus');
  await expect(page.getByRole('heading', { name: 'Session expirée' })).toBeVisible();
  await expect(page.getByRole('link', { name: 'Se reconnecter' })).toBeVisible();
});
