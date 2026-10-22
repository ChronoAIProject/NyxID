/** Cleanup for an already-invalidated login must never resolve stored
 * credentials, refresh, or adopt a successor login. */
export async function unregisterCapturedDevice(url: string, body: unknown, accessToken: string,
  expiresAt: number | undefined, send: typeof fetch = fetch): Promise<void> {
  if (!accessToken || !expiresAt || expiresAt <= Date.now()) return;
  const controller = new AbortController();
  const timer = setTimeout(() => controller.abort(), 10000);
  try {
    const response = await send(url, {
      method: 'DELETE', headers: { Accept: 'application/json', 'Content-Type': 'application/json',
        Authorization: `Bearer ${accessToken}` }, body: JSON.stringify(body), signal: controller.signal,
    });
    if (!response.ok) throw new Error('push_unregister_failed');
  } finally { clearTimeout(timer); }
}
