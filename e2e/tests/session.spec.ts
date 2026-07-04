import { test, expect } from '@playwright/test';

test.describe('Session lifecycle', () => {
  test('landing page renders the host entry point', async ({ page }) => {
    await page.goto('/');

    await expect(page.getByRole('heading', { name: 'Presto-Matic' })).toBeVisible();
    await expect(page.getByRole('button', { name: 'Créer une session (host)' })).toBeVisible();
  });

  test('HTTP session response carries workspace identity facts', async ({ request }) => {
    const response = await request.post('/sessions');
    expect(response.ok()).toBeTruthy();

    const body = await response.json();
    const data = body.data;
    expect(data.session_id).toBeTruthy();
    expect(data.tenant_id).toBe('tenant_local');
    expect(data.workspace_id).toBe(`workspace_${data.session_id}`);
    expect(data.workspace_identity.tenant_id).toBe('tenant_local');
    expect(data.workspace_identity.workspace_id).toBe(data.workspace_id);
    expect(data.workspace_identity.role_assignments[0].role).toBe('host');
    expect(data.workspace_identity.role_assignments[0].actor_ref.actor_type).toBe('human');
  });

  test('host creates a session, participant answers, host reveals leaderboard', async ({ browser }) => {
    const host = await browser.newPage();
    const participant = await browser.newPage();

    await host.goto('/');
    await host.getByRole('button', { name: 'Créer une session (host)' }).click();

    const sessionCode = host.locator('#code');
    await expect(sessionCode).not.toHaveText('');

    const joinHref = await host.locator('#joinlink').getAttribute('href');
    expect(joinHref).toContain('/?s=');
    await expect(host.locator('#log')).toContainText('connecté');

    await participant.goto(joinHref!);
    await expect(participant.locator('#join-code')).not.toHaveText('');
    await participant.locator('#name').fill('Alice');
    await participant.getByRole('button', { name: 'Rejoindre' }).click();
    await expect(participant.locator('#log')).toContainText('connecté');
    await expect(host.locator('#hoststatus')).toContainText('1 participant');

    await host.getByRole('button', { name: 'Ouvrir une question' }).click();
    await expect(participant.locator('#question')).toHaveText('Capital of France?');
    await expect(participant.locator('#grounding')).toContainText('Question sourcée');
    await expect(participant.locator('#grounding')).toContainText('fixture de démonstration');
    await expect(participant.locator('#grounding')).toContainText('refs privées');

    await participant.getByRole('button', { name: 'Paris' }).click();
    await expect(host.locator('#log')).toContainText('réponse reçue');

    await host.getByRole('button', { name: 'Révéler' }).click();
    await expect(participant.locator('#leaderboard')).toBeVisible();
    await expect(participant.locator('#board')).toContainText('Alice');
    const leaderboardText = (await participant.locator('#board').textContent()) ?? '';
    const score = Number(leaderboardText.match(/Alice — (\d+)/)?.[1] ?? '0');
    expect(score).toBeGreaterThanOrEqual(500);

    await host.close();
    await participant.close();
  });
});
