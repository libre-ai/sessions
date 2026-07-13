import { expect, test, type Page } from '@playwright/test';

function projectContextOptions() {
  const { baseURL: _baseURL, trace: _trace, video: _video, screenshot: _screenshot, browserName: _browserName, ...options } = test.info().project.use as Record<string, unknown>;
  return options;
}

function collectWebSocketFrames(page: Page) {
  const frames: string[] = [];
  page.on('websocket', ws => {
    ws.on('framereceived', frame => {
      const payload = typeof frame.payload === 'string' ? frame.payload : String(frame.payload);
      frames.push(payload);
    });
  });
  return frames;
}

async function createHostSession(page: Page) {
  await page.goto('/');
  const responsePromise = page.waitForResponse(response =>
    response.request().method() === 'POST'
      && /\/sessions$/.test(response.url())
      && response.status() === 200,
  );
  await page.getByRole('button', { name: 'Créer une session (host)' }).click();
  const response = await responsePromise;
  const body = await response.json();
  await expect(page.locator('#secure-joinlink')).toHaveAttribute('href', /^\/join\/[A-Z0-9]{6,12}#token=/);
  const secureHref = await page.locator('#secure-joinlink').getAttribute('href');
  expect(secureHref).toBeTruthy();
  const sessionId = secureHref!.match(/\/join\/([A-Z0-9]{6,12})/)?.[1];
  expect(sessionId).toBeTruthy();
  return { secureHref: secureHref!, sessionId: sessionId!, hostToken: body.data.host_token as string };
}

async function joinSecureSession(page: Page, secureHref: string, name: string) {
  await page.goto(secureHref);
  await expect.poll(() => page.evaluate(() => location.hash)).toBe('');
  await page.locator('#join-name').fill(name);
  const responsePromise = page.waitForResponse(response =>
    response.request().method() === 'POST'
      && /\/join\/[A-Z0-9]{6,12}\/participants$/.test(response.url())
      && response.status() === 200,
  );
  await page.getByRole('button', { name: 'Rejoindre' }).click();
  const response = await responsePromise;
  const body = await response.json();
  return body.data.participant_token as string;
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

async function sendRawWebSocketMessage(page: Page, sessionId: string, token: string, message: object) {
  return page.evaluate(async ({ sessionId, token, message }) => {
    const scheme = location.protocol === 'https:' ? 'wss' : 'ws';
    const url = `${scheme}://${location.host}/ws/${sessionId}?token=${encodeURIComponent(token)}`;
    return await new Promise<void>((resolve, reject) => {
      const ws = new WebSocket(url);
      const timer = setTimeout(() => reject(new Error('timeout')), 5000);
      ws.onopen = () => {
        ws.send(JSON.stringify(message));
        setTimeout(() => {
          clearTimeout(timer);
          ws.close();
          resolve();
        }, 100);
      };
      ws.onerror = () => {
        clearTimeout(timer);
        reject(new Error('websocket error'));
      };
    });
  }, { sessionId, token, message });
}

async function sendGuestWebSocketMessage(page: Page, sessionId: string, token: string) {
  return page.evaluate(async ({ sessionId, token }) => {
    const scheme = location.protocol === 'https:' ? 'wss' : 'ws';
    const url = `${scheme}://${location.host}/ws/${sessionId}?token=${encodeURIComponent(token)}`;
    return await new Promise<string>((resolve, reject) => {
      const ws = new WebSocket(url);
      const timer = setTimeout(() => reject(new Error('timeout')), 5000);
      ws.onopen = () => ws.send(JSON.stringify({ type: 'reveal' }));
      ws.onmessage = event => {
        try {
          const msg = JSON.parse(String(event.data)) as { type?: string; reason?: string };
          if (msg.type === 'error') {
            clearTimeout(timer);
            ws.close();
            resolve(msg.reason ?? '');
          }
        } catch (error) {
          clearTimeout(timer);
          reject(error);
        }
      };
      ws.onerror = () => {
        clearTimeout(timer);
        reject(new Error('websocket error'));
      };
    });
  }, { sessionId, token });
}

test('secure guest flow keeps the hash scrubbed, rejects host-only guest messages, late-joins asking/revealed, and survives reconnect', async ({ browser }) => {
  const options = projectContextOptions();
  const hostContext = await browser.newContext(options);
  const guestContext = await browser.newContext(options);
  const host = await hostContext.newPage();
  const guest1 = await guestContext.newPage();
  const guest2 = await guestContext.newPage();
  const guest3 = await guestContext.newPage();

  const frames1 = collectWebSocketFrames(guest1);
  const frames2 = collectWebSocketFrames(guest2);
  const frames3 = collectWebSocketFrames(guest3);

  const { secureHref, sessionId, hostToken } = await createHostSession(host);
  const participantToken = await joinSecureSession(guest1, secureHref, 'Alice');

  const storage = await guest1.evaluate(async () => ({
    local: localStorage.length,
    session: sessionStorage.length,
    caches: await caches.keys(),
    sw: (await navigator.serviceWorker.getRegistrations()).length,
  }));
  expect(storage.local).toBe(0);
  expect(storage.session).toBe(0);
  expect(storage.caches).toEqual([]);
  expect(storage.sw).toBe(0);

  await guest2.goto(secureHref);
  await expect.poll(() => guest2.evaluate(() => location.hash)).toBe('');
  await guest2.locator('#join-name').fill('Bob');
  await guest2.getByRole('button', { name: 'Rejoindre' }).click();
  await expect(guest2.locator('body')).toContainText('Lobby');

  await sendRawWebSocketMessage(host, sessionId, hostToken, {
    type: 'push_question',
    question: {
      id: 'q1',
      text: '2 + 2 ?',
      kind: 'single',
      choices: ['3', '4'],
      correct_choices: [1],
      source_section_ids: ['doc1#s1'],
      timer_sec: 30,
    },
  });
  await expect.poll(() => frames1.some(frame => frame.includes('"type":"question_opened"'))).toBeTruthy();
  await expect(guest1.getByRole('heading', { name: 'Question' })).toBeVisible();
  await expect(guest2.getByRole('heading', { name: 'Question' })).toBeVisible({ timeout: 10000 });
  expect(frames2.some(frame => frame.includes('"type":"question_opened"'))).toBe(true);
  expect(frames2.some(frame => frame.includes('correct_choices'))).toBe(false);

  const questionText = await guest1.locator('.presto-card__body').first().textContent() ?? '';
  const answer = pickAnswer(questionText);
  await guest1.getByLabel(answer).click();
  await guest1.getByRole('button', { name: 'Valider' }).click();
  await expect(host.locator('#log')).toContainText('réponse reçue');

  const hostOnlyReason = await sendGuestWebSocketMessage(guest1, sessionId, participantToken);
  expect(hostOnlyReason).toBe('host only');

  await sendRawWebSocketMessage(host, sessionId, hostToken, { type: 'reveal' });
  await expect(guest1.getByRole('heading', { name: 'Révélation' })).toBeVisible();
  await expect(guest2.getByRole('heading', { name: 'Révélation' })).toBeVisible();
  expect(frames1.some(frame => frame.includes('"type":"answers_revealed"') && frame.includes('correct_choices'))).toBe(true);
  expect(frames2.some(frame => frame.includes('"type":"answers_revealed"') && frame.includes('correct_choices'))).toBe(true);

  await expect(guest1.getByRole('heading', { name: 'Classement' })).toBeVisible();
  const scoreBefore = (await guest1.locator('.presto-list li').first().textContent()) ?? '';
  await guestContext.setOffline(true);
  await guest1.evaluate(() => window.dispatchEvent(new Event('offline')));
  await guestContext.setOffline(false);
  await guest1.evaluate(() => window.dispatchEvent(new Event('online')));
  await expect.poll(async () => (await guest1.locator('.presto-list li').first().textContent()) ?? '').toBe(scoreBefore);

  await guest3.goto(secureHref);
  await expect.poll(() => guest3.evaluate(() => location.hash)).toBe('');
  await guest3.locator('#join-name').fill('Cara');
  await guest3.getByRole('button', { name: 'Rejoindre' }).click();
  await expect(guest3.getByRole('heading', { name: 'Révélation' })).toBeVisible({ timeout: 10000 });

  await hostContext.close();
  await guestContext.close();
});

test('invalid or malformed secure fragments are scrubbed immediately and fail bounded', async ({ browser }) => {
  const options = projectContextOptions();
  const hostContext = await browser.newContext(options);
  const host = await hostContext.newPage();
  const { secureHref } = await createHostSession(host);
  const base = secureHref.replace(/#token=.*$/, '');

  const invalidTokenPage = await browser.newPage();
  await invalidTokenPage.goto(`${base}#token=BADCODE`);
  await expect.poll(() => invalidTokenPage.evaluate(() => location.hash)).toBe('');
  await invalidTokenPage.locator('#join-name').fill('Zoe');
  await invalidTokenPage.getByRole('button', { name: 'Rejoindre' }).click();
  await expect(invalidTokenPage.getByRole('heading', { name: 'Lien expiré' })).toBeVisible();
  await expect(invalidTokenPage.locator('.presto-card__body')).toContainText(/expiré|refusé/i);
  await invalidTokenPage.close();

  const malformedFragments = ['#token=', '#oops=1'];
  for (const fragment of malformedFragments) {
    const page = await browser.newPage();
    await page.goto(`${base}${fragment}`);
    await expect.poll(() => page.evaluate(() => location.hash)).toBe('');
    await expect(page.getByRole('heading', { name: 'Lien invalide' })).toBeVisible();
    await expect(page.locator('.presto-card__body')).toContainText(/invalide|token manquant/i);
    await page.close();
  }

  await hostContext.close();
});
