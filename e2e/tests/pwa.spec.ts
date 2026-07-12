import { test, expect } from '@playwright/test';

async function waitForControl(page: import('@playwright/test').Page) {
  await page.goto('/app');
  await page.evaluate(async () => {
    const registration = await navigator.serviceWorker.register('/app/sw.js', {
      scope: '/app/', updateViaCache: 'none',
    });
    if (registration.active) return;
    const worker = registration.installing ?? registration.waiting;
    if (!worker) throw new Error('service worker did not start installing');
    if (worker.state === 'activated') return;
    await new Promise<void>((resolve, reject) => worker.addEventListener('statechange', () => {
      if (worker.state === 'activated') resolve();
      if (worker.state === 'redundant') reject(new Error('service worker became redundant'));
    }));
  });
  if (!await page.evaluate(() => Boolean(navigator.serviceWorker.controller))) {
    await page.reload();
  }
  await expect.poll(() => page.evaluate(() => Boolean(navigator.serviceWorker.controller))).toBe(true);
}

test('manifest, PNG icons and Chromium installability metadata are coherent', async ({ page, browserName }) => {
  await page.goto('/app');
  const manifest = await page.evaluate(async () => (await fetch('/app/manifest.webmanifest')).json());
  expect(manifest).toMatchObject({ id: '/app/', scope: '/app/', start_url: '/app/', display: 'standalone', lang: 'fr' });
  expect(manifest.icons).toHaveLength(4);
  for (const icon of manifest.icons) {
    const dimensions = await page.evaluate(async (src: string) => {
      const image = new Image();
      image.src = src;
      await image.decode();
      return [image.naturalWidth, image.naturalHeight];
    }, icon.src);
    const size = Number(icon.sizes.split('x')[0]);
    expect(dimensions).toEqual([size, size]);
    expect((await page.request.get(icon.src)).headers()['content-type']).toBe('image/png');
  }

  if (browserName === 'chromium') {
    const session = await page.context().newCDPSession(page);
    const result = await session.send('Page.getAppManifest');
    expect(result.url).toContain('/app/manifest.webmanifest');
    expect(result.errors).toEqual([]);
    expect(result.data).toContain('"display":"standalone"');
  }
});

test('Cache Storage is an exact shell allowlist and APIs stay network-only offline', async ({ page, context }) => {
  await waitForControl(page);
  const internal = await page.evaluate(async () => (await fetch('/app/owner-shell-manifest.json')).json());
  const expected = internal.precache.map((entry: { url: string }) => new URL(entry.url, page.url()).href).sort();
  const initial = await page.evaluate(async () => {
    const names = await caches.keys();
    const keys = (await Promise.all(names.map(async name => (await caches.open(name)).keys()))).flat();
    return { names, urls: keys.map(request => request.url).sort() };
  });
  expect(initial.names).toEqual([`rumble-owner-shell-v1-${internal.bundle_id}`]);
  expect(initial.urls).toEqual(expected);
  expect(initial.urls.some((url: string) => /\/(auth|api|corpus|sessions|ws)\//.test(url))).toBe(false);

  await page.evaluate(async () => {
    await Promise.allSettled([
      fetch('/auth/login'),
      fetch('/api/rag/query', { method: 'POST', body: '{}' }),
      fetch('/corpus/documents?document_id=cache-probe', { method: 'POST', body: 'never cache' }),
      fetch('/sessions', { method: 'POST' }),
      fetch('/api/bearer-probe', { headers: { Authorization: 'Bearer explicit-test-value' } }),
      fetch('http://127.0.0.1:9/third-party'),
    ]);
  });
  const afterSensitiveRequests = await page.evaluate(async () => {
    const names = await caches.keys();
    return (await Promise.all(names.map(async name => (await caches.open(name)).keys())))
      .flat().map(request => request.url).sort();
  });
  expect(afterSensitiveRequests).toEqual(expected);

  await context.setOffline(true);
  await page.goto('/app/notebook');
  await expect(page.getByRole('heading', { name: 'Interroger votre corpus' })).toBeVisible();
  const apiFailed = await page.evaluate(async () => {
    try {
      await fetch('/api/cache-probe');
      return false;
    } catch {
      return true;
    }
  });
  expect(apiFailed).toBe(true);
  await context.setOffline(false);
});
