import { test, expect } from '@playwright/test';
import path from 'node:path';

const uploadFixture = path.resolve(__dirname, '../../crates/server/assets/approved-owner-upload.md');
const enabled = process.env.KEYCLOAK_E2E === '1';
const username = process.env.KEYCLOAK_TEST_USERNAME;
const password = process.env.KEYCLOAK_TEST_PASSWORD;

test.use({
  viewport: { width: 390, height: 844 },
  isMobile: true,
  hasTouch: true,
  // Credentials and the protocol callback must not be captured in a trace.
  trace: 'off',
});

test.describe('Owner auth against development Keycloak', () => {
  test.describe.configure({ mode: 'serial' });
  test.skip(!enabled, 'manual Keycloak gate; see docs/e2e-testing.md');

  test('callback from browser A is rejected in browser B', async ({ browser, baseURL }) => {
    expect(username, 'KEYCLOAK_TEST_USERNAME is required').toBeTruthy();
    expect(password, 'KEYCLOAK_TEST_PASSWORD is required').toBeTruthy();
    expect(baseURL).toBeTruthy();

    const browserA = await browser.newContext();
    const browserB = await browser.newContext();
    const pageA = await browserA.newPage();
    const pageB = await browserB.newPage();
    try {
      await pageA.route('**/auth/callback?**', (route) => route.abort());
      await pageA.goto(`${baseURL}/app/login`);
      await pageA.getByRole('link', { name: 'Continuer vers la connexion' }).click();
      await pageA.locator('#username').fill(username!);
      await pageA.locator('#password').fill(password!);
      const callbackRequest = pageA.waitForRequest((request) =>
        request.url().startsWith(`${baseURL}/auth/callback?`),
      );
      await pageA.locator('#kc-login').click();
      const callbackUrl = (await callbackRequest).url();

      const swapped = await pageB.goto(callbackUrl);
      expect(swapped?.status()).toBe(401);
      expect((await browserB.cookies()).some(
        (cookie) => cookie.name === '__Host-rumble_session',
      )).toBeFalsy();
    } finally {
      await browserA.close();
      await browserB.close();
    }
  });

  test('login → personal space → real notebook answer → refresh → logout', async ({ page }) => {
    expect(username, 'KEYCLOAK_TEST_USERNAME is required').toBeTruthy();
    expect(password, 'KEYCLOAK_TEST_PASSWORD is required').toBeTruthy();

    await page.goto('/app/login');
    await page.getByRole('link', { name: 'Continuer vers la connexion' }).click();
    await page.locator('#username').fill(username!);
    await page.locator('#password').fill(password!);
    await page.locator('#kc-login').click();
    await expect(page).toHaveURL(/\/app\/?$/);

    const me = await page.evaluate(async () => {
      const response = await fetch('/api/me', { credentials: 'include' });
      return { status: response.status, body: await response.json() };
    });
    expect(me.status).toBe(200);
    expect(me.body.data.actor_id).toMatch(/^actor_/);
    expect(me.body.data.personal_space_id).toMatch(/^space_/);
    expect(JSON.stringify(me.body)).not.toMatch(/token|nonce|verifier|email|external-subject/i);

    const current = await page.evaluate(async () => {
      const response = await fetch('/api/spaces/current', { credentials: 'include' });
      return { status: response.status, body: await response.json() };
    });
    expect(current.status).toBe(200);
    expect(current.body.data.space.id).toBe(me.body.data.personal_space_id);
    expect(current.body.data.space.role).toBe('owner');
    expect(current.body.data.space.capabilities).toContain('delete_space');

    const authCookie = (await page.context().cookies()).find(
      (cookie) => cookie.name === '__Host-rumble_session',
    );
    expect(authCookie).toMatchObject({
      httpOnly: true,
      secure: true,
      sameSite: 'Strict',
      path: '/',
    });
    expect(await page.evaluate(() => document.cookie)).not.toContain('__Host-rumble_session');
    expect(await page.evaluate(() => ({
      local: localStorage.length,
      session: sessionStorage.length,
    }))).toEqual({ local: 0, session: 0 });

    await page.reload();
    const refreshed = await page.evaluate(async () => (await fetch('/api/me')).status);
    expect(refreshed).toBe(200);

    // No API route is mocked in this spec: this traverses the real cookie auth,
    // current-space handler and retrieve → generate → verify → permit gate.
    await page.goto('/app/notebook');
    await expect(page.getByRole('heading', { name: 'Prêt à interroger les claims approuvés' })).toBeVisible();
    await page.getByLabel('Question au corpus').fill('Quelle est la capitale de la France ?');
    await page.getByRole('button', { name: 'Envoyer' }).click();
    await expect(page.getByRole('heading', { name: 'Réponse', exact: true })).toBeVisible();
    await expect(page.getByText('Paris est la capitale de la France.')).toBeVisible();
    const citations = page.getByRole('region', { name: 'Citations approuvées' });
    await expect(citations).toBeVisible();
    await expect(citations.locator('.presto-source-card')).toHaveCount(1);
    await expect(citations.locator('.presto-source-card')).toContainText('Référence géographique approuvée');

    // Real browser FileData → JSON upload → process-local store → approved permit.
    await page.goto('/app/corpus');
    await page.getByLabel('Choisir exactement un document').setInputFiles(uploadFixture);
    await expect(page.getByText('Sélectionné : approved-owner-upload.md')).toBeVisible();
    await page.getByRole('button', { name: 'Ajouter le document' }).click();
    await expect(page.getByText(/Approved — correspondance exacte/)).toBeVisible();

    await page.goto('/app/notebook');
    await page.getByLabel('Question au corpus').fill('Quel est le statut des uploads arbitraires ?');
    await page.getByRole('button', { name: 'Envoyer' }).click();
    await expect(page.getByText('Les uploads arbitraires restent Pending et ne sont jamais utilisés pour une réponse Grounded.')).toBeVisible();
    const uploadCitation = page.getByRole('region', { name: 'Citations approuvées' });
    await expect(uploadCitation).toContainText('Politique approuvée des uploads owner');
    await expect(uploadCitation).not.toContainText('approved-owner-upload.md');

    await page.goto('/app/corpus');
    await page.reload();
    await expect(page.getByRole('list', { name: 'Documents du corpus' })).toContainText('approved-owner-upload.md');
    await expect(page.getByRole('list', { name: 'Documents du corpus' })).toContainText('Approved');

    await page.goto('/app/settings');
    await page.getByRole('button', { name: 'Se déconnecter' }).click();
    await expect(page).toHaveURL(/\/app\/login\/?$/);
    expect(await page.evaluate(async () => (await fetch('/api/me')).status)).toBe(401);
    expect((await page.context().cookies()).some(
      (cookie) => cookie.name === '__Host-rumble_session',
    )).toBeFalsy();
  });
});
