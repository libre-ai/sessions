import { expect, test, type Page } from '@playwright/test';

function projectContextOptions() {
  const {
    baseURL: _baseURL,
    trace: _trace,
    video: _video,
    screenshot: _screenshot,
    browserName: _browserName,
    ...options
  } = test.info().project.use as Record<string, unknown>;
  return options;
}

type FrameMessage = {
  type?: string;
  snapshot?: {
    phase?: string;
    answered?: boolean;
  };
};

function collectWebSocketActivity(page: Page) {
  let frames: string[] = [];
  let sockets = 0;

  page.on('websocket', ws => {
    sockets += 1;
    ws.on('framereceived', frame => {
      frames = frames.concat(typeof frame.payload === 'string' ? frame.payload : String(frame.payload));
    });
  });

  return {
    get frames() {
      return frames;
    },
    get socketCount() {
      return sockets;
    },
  };
}

function parseFrame(frame: string): FrameMessage | null {
  try {
    return JSON.parse(frame) as FrameMessage;
  } catch {
    return null;
  }
}

function countFrames(frames: string[], predicate: (frame: string) => boolean) {
  return frames.filter(predicate).length;
}

function countMessageType(frames: string[], type: string) {
  return countFrames(frames, frame => parseFrame(frame)?.type === type);
}

function countSnapshots(frames: string[], phase: 'asking' | 'revealed', answered?: boolean) {
  return countFrames(frames, frame => {
    const message = parseFrame(frame);
    return (
      message?.type === 'snapshot'
      && message.snapshot?.phase === phase
      && (answered === undefined || message.snapshot.answered === answered)
    );
  });
}

async function waitForStatus(page: Page, text: string) {
  await expect(page.locator('.join-shell > .presto-toast')).toContainText(text);
}

async function createHostSession(page: Page) {
  await page.goto('/');
  await page.getByRole('button', { name: 'Créer une session (host)' }).click();
  await expect(page.locator('#secure-joinlink')).toHaveAttribute('href', /^\/join\/[A-Z0-9]{6,12}#token=/);
  const secureHref = await page.locator('#secure-joinlink').getAttribute('href');
  expect(secureHref).toBeTruthy();
  await expect(page.locator('#log')).toContainText('connecté');
  return secureHref!;
}

async function joinSecureSession(page: Page, secureHref: string, name: string) {
  await page.goto(secureHref);
  await expect.poll(() => page.evaluate(() => location.hash)).toBe('');
  await expect.poll(() => page.evaluate(() => document.querySelectorAll('style').length)).toBe(0);
  await expect(page.locator('.join-shell')).toHaveCSS('display', 'grid');
  await expect(page.getByRole('heading', { name: 'Rejoindre une session' })).toBeVisible();
  await page.locator('#join-name').fill(name);
  const responsePromise = page.waitForResponse(response =>
    response.request().method() === 'POST'
      && /\/join\/[A-Z0-9]{6,12}\/participants$/.test(response.url())
      && response.status() === 200,
  );
  await page.getByRole('button', { name: 'Rejoindre' }).click();
  const response = await responsePromise;
  const body = await response.json();
  await expect(page.getByRole('heading', { name: 'Lobby' })).toBeVisible();
  return body.data.participant_token as string;
}

async function attemptGuestReveal(page: Page, sessionId: string, token: string) {
  return page.evaluate(async ({ sessionId, token }) => {
    const scheme = location.protocol === 'https:' ? 'wss' : 'ws';
    const url = `${scheme}://${location.host}/ws/${sessionId}?token=${encodeURIComponent(token)}`;
    return await new Promise<string>((resolve, reject) => {
      const socket = new WebSocket(url);
      const timeout = window.setTimeout(() => reject(new Error('guest host-action timeout')), 5000);
      socket.onopen = () => socket.send(JSON.stringify({ type: 'reveal' }));
      socket.onmessage = event => {
        const message = JSON.parse(String(event.data)) as { type?: string; reason?: string };
        if (message.type === 'error') {
          window.clearTimeout(timeout);
          socket.close();
          resolve(message.reason ?? '');
        }
      };
      socket.onerror = () => {
        window.clearTimeout(timeout);
        reject(new Error('guest host-action websocket failed'));
      };
    });
  }, { sessionId, token });
}

type QuestionOpenedFrame = {
  type?: string;
  question?: {
    text?: string;
  };
};

type AnswersRevealedFrame = {
  type?: string;
  leaderboard?: Array<{
    name?: string;
    score?: number;
  }>;
};

function latestMessage<T extends { type?: string }>(frames: string[], type: string) {
  for (let index = frames.length - 1; index >= 0; index -= 1) {
    const message = parseFrame(frames[index]) as T | null;
    if (message?.type === type) {
      return message;
    }
  }
  return null;
}

function pickAnswer(questionText: string): string {
  const text = questionText.toLowerCase();
  if (text.includes('capital of france') || text.includes('paris')) return 'Paris';
  if (text.includes('ownership system prevent')) return 'Data races and use-after-free';
  if (text.includes('many owners can a value have')) return 'Exactly one';
  if (text.includes('goes out of scope')) return 'Rust calls drop() automatically';
  if (text.includes('2+2') || text.includes('2 + 2')) return '4';
  return 'Paris';
}

function revealedLeaderboardFromFrames(frames: string[]) {
  const message = latestMessage<AnswersRevealedFrame>(frames, 'answers_revealed');
  return (message?.leaderboard ?? []).map(entry => `${entry.name ?? ''} — ${entry.score ?? 0}`);
}

test('secure guest reconnect restores answered and revealed snapshots and supports late join', async ({ browser }) => {
  const options = projectContextOptions();
  const hostContext = await browser.newContext(options);
  const guestContext = await browser.newContext(options);
  const peerContext = await browser.newContext(options);
  const host = await hostContext.newPage();
  const alice = await guestContext.newPage();
  const bob = await peerContext.newPage();
  const lateJoiner = await guestContext.newPage();

  const aliceActivity = collectWebSocketActivity(alice);
  const lateActivity = collectWebSocketActivity(lateJoiner);

  const secureHref = await createHostSession(host);
  const sessionId = secureHref.match(/\/join\/([A-Z0-9]{6,12})/)?.[1];
  expect(sessionId).toBeTruthy();
  await joinSecureSession(bob, secureHref, 'Bob');
  const participantToken = await joinSecureSession(alice, secureHref, 'Alice');

  await expect.poll(() => aliceActivity.socketCount).toBe(1);
  const storage = await alice.evaluate(async () => ({
    local: localStorage.length,
    session: sessionStorage.length,
    caches: await caches.keys(),
    sw: (await navigator.serviceWorker.getRegistrations()).length,
  }));
  expect(storage.local).toBe(0);
  expect(storage.session).toBe(0);
  expect(storage.caches).toEqual([]);
  expect(storage.sw).toBe(0);

  await host.getByRole('button', { name: 'Ouvrir une question' }).click();
  await expect(alice.getByRole('heading', { name: 'Question' })).toBeVisible();
  await expect(bob.getByRole('heading', { name: 'Question' })).toBeVisible();

  await expect.poll(() => countMessageType(aliceActivity.frames, 'question_opened')).toBeGreaterThan(0);
  const questionMessage = latestMessage<QuestionOpenedFrame>(aliceActivity.frames, 'question_opened');
  const questionText = questionMessage?.question?.text ?? '';
  const answer = pickAnswer(questionText);
  await alice.getByLabel(answer).click();
  await alice.getByRole('button', { name: 'Valider' }).click();

  await expect.poll(() => countMessageType(aliceActivity.frames, 'answer_accepted')).toBeGreaterThan(0);
  await expect(host.locator('#log')).toContainText('réponse reçue');
  expect(aliceActivity.frames.some(frame => frame.includes('correct_choices'))).toBe(false);
  expect(await attemptGuestReveal(host, sessionId!, participantToken)).toBe('host only');
  await expect(alice.getByRole('button', { name: 'Valider' })).toBeDisabled();
  await expect(alice.locator('.presto-question-set')).toHaveAttribute('disabled', 'true');

  const askingSocketsBaseline = aliceActivity.socketCount;
  const askingSnapshotsBaseline = countSnapshots(aliceActivity.frames, 'asking', true);
  const askingResume = alice.waitForResponse(response =>
    /\/sessions\/[A-Z0-9]{6,12}\/participants\/resume$/.test(response.url()),
  );
  await alice.getByRole('button', { name: 'Reprendre la connexion' }).click();
  await waitForStatus(alice, 'Connexion perdue');
  expect((await askingResume).status()).toBe(204);
  await expect.poll(() => aliceActivity.socketCount, { timeout: 30000 }).toBeGreaterThan(askingSocketsBaseline);
  await expect.poll(() => countSnapshots(aliceActivity.frames, 'asking', true), { timeout: 30000 }).toBeGreaterThan(askingSnapshotsBaseline);
  const resumedAskingSnapshot = latestMessage<FrameMessage>(aliceActivity.frames, 'snapshot');
  expect(resumedAskingSnapshot?.snapshot?.phase).toBe('asking');
  expect(resumedAskingSnapshot?.snapshot?.answered).toBe(true);
  await expect(alice.getByRole('heading', { name: 'Question' })).toBeVisible({ timeout: 15000 });
  await expect(alice.getByRole('button', { name: 'Valider' })).toBeDisabled({ timeout: 15000 });
  await expect(alice.locator('.presto-question-set')).toHaveAttribute('disabled', 'true');

  await host.getByRole('button', { name: 'Révéler' }).click();
  await expect.poll(() => countMessageType(aliceActivity.frames, 'answers_revealed')).toBeGreaterThan(0);
  await expect(alice.getByRole('heading', { name: 'Révélation' })).toBeVisible();
  await expect(alice.getByRole('heading', { name: 'Classement' })).toBeVisible();
  const revealedLeaderboard = revealedLeaderboardFromFrames(aliceActivity.frames);
  expect(revealedLeaderboard.length).toBeGreaterThan(0);
  await expect(alice.locator('.presto-list li')).toHaveText(revealedLeaderboard);

  const revealedSocketsBaseline = aliceActivity.socketCount;
  const revealedSnapshotsBaseline = countSnapshots(aliceActivity.frames, 'revealed');
  const answersRevealedBaseline = countMessageType(aliceActivity.frames, 'answers_revealed');
  const revealedResume = alice.waitForResponse(response =>
    /\/sessions\/[A-Z0-9]{6,12}\/participants\/resume$/.test(response.url()),
  );
  await alice.getByRole('button', { name: 'Reprendre la connexion' }).click();
  await waitForStatus(alice, 'Connexion perdue');
  expect((await revealedResume).status()).toBe(204);
  await expect.poll(() => aliceActivity.socketCount, { timeout: 30000 }).toBeGreaterThan(revealedSocketsBaseline);
  await expect.poll(() => countSnapshots(aliceActivity.frames, 'revealed'), { timeout: 30000 }).toBeGreaterThan(revealedSnapshotsBaseline);
  await expect.poll(() => countMessageType(aliceActivity.frames, 'answers_revealed'), { timeout: 30000 }).toBeGreaterThan(answersRevealedBaseline);
  const resumedRevealedSnapshot = latestMessage<FrameMessage>(aliceActivity.frames, 'snapshot');
  expect(resumedRevealedSnapshot?.snapshot?.phase).toBe('revealed');
  await expect(alice.getByRole('heading', { name: 'Révélation' })).toBeVisible({ timeout: 15000 });
  await expect(alice.getByRole('heading', { name: 'Classement' })).toBeVisible();
  await expect(alice.locator('.presto-list li')).toHaveText(revealedLeaderboard);

  await lateJoiner.goto(secureHref);
  await expect.poll(() => lateJoiner.evaluate(() => location.hash)).toBe('');
  await expect(lateJoiner.getByRole('heading', { name: 'Rejoindre une session' })).toBeVisible();
  await lateJoiner.locator('#join-name').fill('Cara');
  await lateJoiner.getByRole('button', { name: 'Rejoindre' }).click();
  await expect.poll(() => lateActivity.socketCount).toBe(1);
  await expect.poll(() => countSnapshots(lateActivity.frames, 'revealed')).toBeGreaterThan(0);
  await expect(lateJoiner.getByRole('heading', { name: 'Révélation' })).toBeVisible();
  await expect(lateJoiner.getByRole('heading', { name: 'Classement' })).toBeVisible();
  await expect(lateJoiner.locator('.presto-list li')).toHaveText(revealedLeaderboard);

  await hostContext.close();
  await guestContext.close();
  await peerContext.close();
});

test('invalid or malformed secure fragments are scrubbed immediately and fail bounded', async ({ browser }) => {
  const options = projectContextOptions();
  const hostContext = await browser.newContext(options);
  const host = await hostContext.newPage();
  const secureHref = await createHostSession(host);
  const base = secureHref.replace(/#token=.*$/, '');

  const invalidTokenPage = await browser.newPage();
  await invalidTokenPage.goto(`${base}#token=BADCODE`);
  await expect.poll(() => invalidTokenPage.evaluate(() => location.hash)).toBe('');
  if (await invalidTokenPage.locator('#join-name').isVisible().catch(() => false)) {
    await invalidTokenPage.locator('#join-name').fill('Zoe');
    await invalidTokenPage.getByRole('button', { name: 'Rejoindre' }).click();
  }
  await expect(invalidTokenPage.locator('.presto-card__body')).toContainText(/expiré|refusé|invalide|session/i);
  await invalidTokenPage.close();

  const malformedFragments = ['#token=', '#oops=1'];
  for (const fragment of malformedFragments) {
    const page = await browser.newPage();
    await page.goto(`${base}${fragment}`);
    await expect.poll(() => page.evaluate(() => location.hash)).toBe('');
    if (await page.locator('#join-name').isVisible().catch(() => false)) {
      await page.locator('#join-name').fill('Zoe');
      await page.getByRole('button', { name: 'Rejoindre' }).click();
    }
    await expect(page.locator('.presto-card__body')).toContainText(/invalide|token manquant|expiré|session/i);
    await page.close();
  }

  await hostContext.close();
});
