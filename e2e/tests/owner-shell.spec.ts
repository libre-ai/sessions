import { test, expect } from '@playwright/test';

test.use({
  viewport: { width: 390, height: 844 },
  isMobile: true,
  hasTouch: true,
});

test.describe('Owner mobile shell', () => {
  test('opens /app and navigates every owner screen without a backend session', async ({ page }) => {
    const pageErrors: string[] = [];
    page.on('pageerror', (error) => pageErrors.push(error.message));

    await page.goto('/app');

    await expect(page.getByRole('heading', { name: 'Travaillez depuis vos propres sources.' })).toBeVisible();
    await expect(page.getByRole('status')).toContainText('cookie HttpOnly');

    let nav = page.getByRole('navigation', { name: 'Navigation owner' });
    await expect(nav).toBeVisible();
    await expect(nav.getByRole('link', { name: 'Accueil' })).toHaveAttribute('aria-current', 'page');

    await nav.getByRole('link', { name: 'Chat RAG' }).click();
    await expect(page).toHaveURL(/\/app\/notebook$/);
    await expect(page.getByRole('heading', { name: 'Interroger votre corpus' })).toBeVisible();
    await expect(page.getByLabel('Question au corpus')).toBeDisabled();
    await expect(page.locator('.owner-query')).toHaveCSS('position', 'sticky');

    nav = page.getByRole('navigation', { name: 'Navigation owner' });
    await nav.getByRole('link', { name: 'Corpus' }).click();
    await expect(page).toHaveURL(/\/app\/corpus$/);
    await expect(page.getByRole('heading', { name: 'Corpus', exact: true })).toBeVisible();
    await expect(page.getByLabel('Choisir exactement un document')).toHaveCount(0);

    nav = page.getByRole('navigation', { name: 'Navigation owner' });
    await nav.getByRole('link', { name: 'Réglages' }).click();
    await expect(page).toHaveURL(/\/app\/settings$/);
    await expect(page.getByRole('heading', { name: 'Réglages' })).toBeVisible();

    await expect(page.getByRole('button', { name: 'Se déconnecter' })).toBeVisible();

    await page.goto('/app/login');
    await expect(page).toHaveURL(/\/app\/login$/);
    await expect(page.getByRole('heading', { name: 'Connexion' })).toBeVisible();
    await expect(page.getByText(/Authorization Code \+ PKCE/)).toBeVisible();

    const browserState = await page.evaluate(async () => ({
      localStorage: localStorage.length,
      sessionStorage: sessionStorage.length,
      serviceWorkers: 'serviceWorker' in navigator
        ? (await navigator.serviceWorker.getRegistrations()).length
        : 0,
    }));
    expect(browserState).toEqual({ localStorage: 0, sessionStorage: 0, serviceWorkers: 0 });
    expect(pageErrors).toEqual([]);
  });

  test('serves nested owner routes directly through the /app fallback', async ({ page }) => {
    await page.goto('/app/settings');
    await expect(page.getByRole('heading', { name: 'Réglages' })).toBeVisible();
    await expect(page.getByRole('navigation', { name: 'Navigation owner' })).toBeVisible();
  });
});
