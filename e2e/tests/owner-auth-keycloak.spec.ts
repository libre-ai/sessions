import { test, expect } from '@playwright/test';

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
  test.skip(!enabled, 'manual Keycloak gate; see docs/e2e-testing.md');

  test('login → me → personal space → refresh → logout', async ({ page }) => {
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

    await page.goto('/app/settings');
    await page.getByRole('button', { name: 'Se déconnecter' }).click();
    await expect(page).toHaveURL(/\/app\/login\/?$/);
    expect(await page.evaluate(async () => (await fetch('/api/me')).status)).toBe(401);
    expect((await page.context().cookies()).some(
      (cookie) => cookie.name === '__Host-rumble_session',
    )).toBeFalsy();
  });
});
