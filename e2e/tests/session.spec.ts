import { test, expect } from '@playwright/test';

test.describe('Session lifecycle', () => {
  test.beforeEach(async ({ context }) => {
    // Set up auth context for host/participant.
    // (In real usage, mint Biscuit token via API.)
  });

  test('Host can join and create a session', async ({ page }) => {
    await page.goto('/');
    await page.click('button:has-text("Create Session")');

    // Verify session created.
    const sessionId = await page
      .locator('[data-testid="session-id"]')
      .textContent();
    expect(sessionId).toBeTruthy();
  });

  test('Participant can join with valid token', async ({ page, browser }) => {
    // Host creates session.
    const hostPage = await browser.newPage();
    await hostPage.goto('/');
    await hostPage.click('button:has-text("Create Session")');
    const joinLink = await hostPage
      .locator('[data-testid="join-link"]')
      .getAttribute('href');

    // Participant joins via link.
    await page.goto(joinLink!);
    const participantId = await page
      .locator('[data-testid="participant-id"]')
      .textContent();
    expect(participantId).toBeTruthy();
  });

  test('Participant can submit answer', async ({ page, browser }) => {
    // Setup: host creates, participant joins.
    const hostPage = await browser.newPage();
    await hostPage.goto('/');
    await hostPage.click('button:has-text("Create Session")');
    const joinLink = await hostPage
      .locator('[data-testid="join-link"]')
      .getAttribute('href');

    await page.goto(joinLink!);

    // Participant submits answer.
    await page.click('text=Option A');
    await page.click('button:has-text("Submit")');

    // Verify submission confirmed.
    await expect(page.locator('text=Answer submitted')).toBeVisible();
  });

  test('Host can reveal answers and see scores', async ({ page, browser }) => {
    // Setup: host creates, participants join + submit.
    const hostPage = await browser.newPage();
    await hostPage.goto('/');
    await hostPage.click('button:has-text("Create Session")');
    const joinLink = await hostPage
      .locator('[data-testid="join-link"]')
      .getAttribute('href');

    const participant1 = await browser.newPage();
    await participant1.goto(joinLink!);
    await participant1.click('text=Option A');
    await participant1.click('button:has-text("Submit")');

    // Host reveals.
    await hostPage.click('button:has-text("Reveal Scores")');

    // Verify leaderboard appears.
    await expect(hostPage.locator('text=Leaderboard')).toBeVisible();
    const scores = await hostPage
      .locator('[data-testid="score"]')
      .allTextContents();
    expect(scores.length).toBeGreaterThan(0);
  });

  test('Error handling: invalid token rejected', async ({ page }) => {
    // Try to join with invalid token.
    await page.goto('/?token=invalid_token_12345');

    // Verify error message.
    await expect(page.locator('text=Invalid or expired token')).toBeVisible();
  });

  test('Leaderboard sorted by score descending', async ({ page, browser }) => {
    // Setup: host creates, multiple participants submit with different elapsed times.
    const hostPage = await browser.newPage();
    await hostPage.goto('/');
    await hostPage.click('button:has-text("Create Session")');
    const joinLink = await hostPage
      .locator('[data-testid="join-link"]')
      .getAttribute('href');

    // Participant 1: fast (5000ms).
    const p1 = await browser.newPage();
    await p1.goto(joinLink!);
    await p1.click('text=Option A');
    await p1.click('button:has-text("Submit")');

    // Participant 2: slower (15000ms).
    const p2 = await browser.newPage();
    await p2.goto(joinLink!);
    await p2.click('text=Option A');
    // Simulate delay... (in real test, use `await page.waitForTimeout(15000)` or clock manipulation)
    await p2.click('button:has-text("Submit")');

    // Host reveals.
    await hostPage.click('button:has-text("Reveal Scores")');

    // Verify p1 (higher score) is first in leaderboard.
    const leaderboardRows = await hostPage
      .locator('[data-testid="leaderboard-row"]')
      .allTextContents();
    const p1Rank = leaderboardRows.findIndex((row) =>
      row.includes('Participant 1'),
    );
    const p2Rank = leaderboardRows.findIndex((row) =>
      row.includes('Participant 2'),
    );
    expect(p1Rank).toBeLessThan(p2Rank);
  });
});
