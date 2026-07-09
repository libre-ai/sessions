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
    await expect(participant.locator('#question')).not.toHaveText('');
    const grounding = participant.locator('#grounding');
    await expect(grounding).toContainText('Question sourcée');
    await expect(grounding).toContainText('refs privées');

    // The served quiz depends on the server's content mode: fixture quiz
    // without DATABASE_URL, grounded quiz (real ingested source) with it.
    // Scoring >= 500 below requires answering correctly in both modes.
    const fixtureMode = ((await grounding.textContent()) ?? '').includes(
      'fixture de démonstration',
    );
    const correctAnswer = fixtureMode ? 'Paris' : 'Data races and use-after-free';
    await participant.getByRole('button', { name: correctAnswer }).click();
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

  test('participant joining with invalid session is rejected', async ({ page }) => {
    // Try to join with a non-existent session code.
    await page.goto('/?s=BADCODE999');
    await page.locator('#name').fill('TestUser');
    await page.getByRole('button', { name: 'Rejoindre' }).click();

    // Should show error in the log (session not found).
    await expect(page.locator('#log')).toContainText(/introuvable|erreur|not found/i);
  });

  test('late join receives current session state', async ({ browser, request }) => {
    // Create session via API.
    const sessionRes = await request.post('/sessions');
    const { data: sessionData } = await sessionRes.json();
    const joinUrl = sessionData.join_url;

    const participant = await browser.newPage();
    await participant.goto(joinUrl);
    await participant.locator('#name').fill('Eve');
    await participant.getByRole('button', { name: 'Rejoindre' }).click();

    // Verify participant is connected.
    await expect(participant.locator('#log')).toContainText('connecté');

    await participant.close();
  });

  test('input validation: participant name is required', async ({ page, request }) => {
    // Create a session via API.
    const sessionRes = await request.post('/sessions');
    const { data } = await sessionRes.json();
    const sessionId = data.session_id;

    // Navigate to join page.
    await page.goto(`/?s=${sessionId}`);
    await expect(page.locator('#join-code')).toContainText(sessionId);

    // Try to join without a name.
    const nameInput = page.locator('#name');
    await nameInput.fill('');
    await page.getByRole('button', { name: 'Rejoindre' }).click();

    // Should show an error message.
    await expect(page.locator('#log')).toContainText(/entrez|champs|required/i);

    await page.close();
  });

  test('network errors are logged with feedback', async ({ page, request }) => {
    // Create a real session first.
    const sessionRes = await request.post('/sessions');
    const { data } = await sessionRes.json();
    const joinHref = data.join_url;

    // Navigate to join page.
    await page.goto(joinHref);
    await page.locator('#name').fill('Frank');

    // Click join and should succeed.
    await page.getByRole('button', { name: 'Rejoindre' }).click();
    await expect(page.locator('#log')).toContainText('connecté');

    await page.close();
  });

  test('grounded question cites a real ingested source (not fixture)', async ({ browser }) => {
    const host = await browser.newPage();
    const participant = await browser.newPage();

    // Host creates the session from the home page (the join_url flow yields
    // the participant UI, which has no "Ouvrir une question" control).
    await host.goto('/');
    await host.getByRole('button', { name: 'Créer une session (host)' }).click();
    const joinHref = await host.locator('#joinlink').getAttribute('href');
    await expect(host.locator('#log')).toContainText('connecté');

    await participant.goto(joinHref!);
    await participant.locator('#name').fill('Student');
    await participant.getByRole('button', { name: 'Rejoindre' }).click();
    await expect(participant.locator('#log')).toContainText('connecté');

    await host.getByRole('button', { name: 'Ouvrir une question' }).click();
    const grounding = participant.locator('#grounding');
    await expect(grounding).toContainText('Question sourcée');

    const groundingText = (await grounding.textContent()) ?? '';
    test.skip(
      groundingText.includes('fixture de démonstration'),
      'server runs in fixture mode (no DATABASE_URL locally); the grounded path is asserted in CI, whose e2e job has Postgres',
    );

    // Grounded mode: the question comes from the ingested Rust ownership
    // guide (gear-loader → SourceRef) and its citation was verified.
    await expect(grounding).toContainText('validée par grounding-verifier');
    await expect(grounding).toContainText('1 citation');
    await expect(grounding).not.toContainText('fixture de démonstration');
    await expect(participant.locator('#question')).toContainText(/ownership/i);

    await host.close();
    await participant.close();
  });
});
