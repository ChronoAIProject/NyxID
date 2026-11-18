import { afterEach, describe, expect, it, vi } from "vitest";
import { NyxIDClient, NyxIDRefreshError, type StorageLike } from "../src/index.js";

function makeClient(fetchFn: typeof fetch, initial = true, providedStorage?: StorageLike): NyxIDClient {
  const values = new Map<string, string>();
  const storage: StorageLike = providedStorage ?? {
    getItem: (key) => values.get(key) ?? null,
    setItem: (key, value) => values.set(key, value),
    removeItem: (key) => values.delete(key),
  };
  const client = new NyxIDClient({
    baseUrl: "https://nyx.example",
    clientId: "client-1",
    redirectUri: "https://app.example/callback",
    fetchFn,
    storage,
  });
  if (initial) {
    storage.setItem(
      "nyxid:tokens:client-1",
      JSON.stringify({
        accessToken: "old-access",
        tokenType: "Bearer",
        expiresIn: 1,
        expiresAt: Date.now() - 1,
        refreshToken: "old-refresh",
      }),
    );
  }
  return client;
}

function response(body: unknown, status = 200, headers = new Headers()): Response {
  return new Response(JSON.stringify(body), { status, headers });
}

afterEach(() => { vi.unstubAllGlobals(); vi.useRealTimers(); });

describe("NyxIDClient refresh lifecycle", () => {
  it("rotates and persists the successor atomically", async () => {
    const fetchFn = vi.fn<typeof fetch>().mockResolvedValue(
      response({ access_token: "new-access", token_type: "Bearer", expires_in: 900, refresh_token: "new-refresh" }),
    );
    const client = makeClient(fetchFn);
    const tokens = await client.refreshToken();
    expect(tokens.accessToken).toBe("new-access");
    expect(client.getStoredTokens()?.refreshToken).toBe("new-refresh");
    expect((fetchFn.mock.calls[0]?.[1] as RequestInit).body).toContain("grant_type=refresh_token");
  });

  it("single-flights concurrent refresh calls", async () => {
    let release!: (value: Response) => void;
    const fetchFn = vi.fn<typeof fetch>().mockReturnValue(new Promise((resolve) => { release = resolve; }));
    const client = makeClient(fetchFn);
    const first = client.refreshToken();
    const second = client.refreshToken();
    expect(fetchFn).toHaveBeenCalledOnce();
    release(response({ access_token: "new", token_type: "Bearer", expires_in: 900 }));
    await expect(Promise.all([first, second])).resolves.toHaveLength(2);
  });

  it("clears tokens for invalid_grant", async () => {
    const fetchFn = vi.fn<typeof fetch>().mockResolvedValue(response({ error: "invalid_grant" }, 400));
    const client = makeClient(fetchFn);
    await expect(client.refreshToken()).rejects.toMatchObject({ code: "invalid_grant", retryable: false });
    expect(client.getStoredTokens()).toBeNull();
  });

  it("keeps tokens and reports retryable failures", async () => {
    const fetchFn = vi.fn<typeof fetch>().mockResolvedValue(response({ error: "temporarily_unavailable" }, 503));
    const client = makeClient(fetchFn);
    await expect(client.refreshToken()).rejects.toBeInstanceOf(NyxIDRefreshError);
    expect(client.getStoredTokens()?.refreshToken).toBe("old-refresh");
  });

  it("honors a numeric Retry-After before retrying", async () => {
    const fetchFn = vi
      .fn<typeof fetch>()
      .mockResolvedValueOnce(
        response(
          { error: "temporarily_unavailable" },
          503,
          new Headers({ "Retry-After": "0" }),
        ),
      )
      .mockResolvedValueOnce(
        response({ access_token: "retried", token_type: "Bearer", expires_in: 900 }),
      );
    const client = makeClient(fetchFn);
    await expect(client.refreshToken()).resolves.toMatchObject({
      accessToken: "retried",
    });
    expect(fetchFn).toHaveBeenCalledTimes(2);
  });

  it("refreshes shortly before expiry through getAccessToken", async () => {
    const fetchFn = vi.fn<typeof fetch>().mockResolvedValue(
      response({ access_token: "fresh", token_type: "Bearer", expires_in: 900 }),
    );
    const client = makeClient(fetchFn);
    await expect(client.getAccessToken()).resolves.toBe("fresh");
    expect(fetchFn).toHaveBeenCalledOnce();
  });
});


describe("refresh concurrency and failure fences", () => {
  function storage(): StorageLike {
    const rows = new Map<string, string>();
    return { getItem: (key) => rows.get(key) ?? null, setItem: (key, value) => { rows.set(key, value); }, removeItem: (key) => { rows.delete(key); } };
  }

  it("single-flights multiple client instances sharing storage in one tab", async () => {
    const shared = storage();
    const fetchFn = vi.fn<typeof fetch>().mockResolvedValue(response({ access_token: "fresh", token_type: "Bearer", expires_in: 900, refresh_token: "rotated" }));
    const first = makeClient(fetchFn, true, shared);
    const second = makeClient(fetchFn, false, shared);
    await Promise.all([first.refreshToken(), second.refreshToken()]);
    expect(fetchFn).toHaveBeenCalledOnce();
  });

  it("does not resurrect a session cleared during the network request", async () => {
    let release!: (response: Response) => void;
    const fetchFn = vi.fn<typeof fetch>().mockReturnValue(new Promise((resolve) => { release = resolve; }));
    const client = makeClient(fetchFn);
    const pending = client.refreshToken();
    client.clearSession();
    release(response({ access_token: "late", token_type: "Bearer", expires_in: 900, refresh_token: "late-refresh" }));
    await expect(pending).rejects.toMatchObject({ code: "session_changed" });
    expect(client.getStoredTokens()).toBeNull();
    expect(fetchFn).toHaveBeenCalledOnce();
  });

  it("retains a replacement login when an older refresh returns invalid_grant", async () => {
    const shared = storage();
    let release!: (response: Response) => void;
    const fetchFn = vi.fn<typeof fetch>().mockReturnValue(new Promise((resolve) => { release = resolve; }));
    const client = makeClient(fetchFn, true, shared);
    const pending = client.refreshToken();
    shared.setItem("nyxid:tokens:client-1", JSON.stringify({ accessToken: "new-login", refreshToken: "new-login-refresh", expiresIn: 900, tokenType: "Bearer" }));
    release(response({ error: "invalid_grant" }, 400));
    await expect(pending).resolves.toMatchObject({ accessToken: "new-login" });
    expect(client.getStoredTokens()?.refreshToken).toBe("new-login-refresh");
  });

  it("uses Web Locks across tabs and adopts the stored successor", async () => {
    const shared = storage();
    let tail = Promise.resolve();
    const request = vi.fn((_name: string, task: () => Promise<unknown>) => {
      const result = tail.then(task);
      tail = result.then(() => undefined, () => undefined);
      return result;
    });
    vi.stubGlobal("navigator", { locks: { request } });
    vi.stubGlobal("window", { localStorage: shared });
    const fetchFn = vi.fn<typeof fetch>().mockResolvedValue(response({ access_token: "fresh", token_type: "Bearer", expires_in: 900, refresh_token: "rotated" }));
    const first = makeClient(fetchFn, true, shared);
    // A distinct storage wrapper models another document sharing persistence.
    const second = makeClient(fetchFn, false, { ...shared });
    await Promise.all([first.refreshToken(), second.refreshToken()]);
    expect(request).toHaveBeenCalledTimes(2);
    expect(fetchFn).toHaveBeenCalledOnce();
    expect(second.getStoredTokens()?.refreshToken).toBe("rotated");
  });

  it("fails closed for shared storage without a cross-tab mutex", async () => {
    const shared = storage();
    vi.stubGlobal("navigator", {});
    vi.stubGlobal("window", { localStorage: shared });
    const fetchFn = vi.fn<typeof fetch>();
    const client = makeClient(fetchFn, true, shared);
    await expect(client.refreshToken()).rejects.toMatchObject({ code: "refresh_coordination_unavailable", retryable: true });
    expect(fetchFn).not.toHaveBeenCalled();
    expect(client.getStoredTokens()?.refreshToken).toBe("old-refresh");
  });

  it("does not retry definitive errors or clear tokens on retryable network errors", async () => {
    const denied = vi.fn<typeof fetch>().mockResolvedValue(response({ error: "invalid_grant" }, 400));
    await expect(makeClient(denied).refreshToken()).rejects.toMatchObject({ code: "invalid_grant" });
    expect(denied).toHaveBeenCalledOnce();
    const network = vi.fn<typeof fetch>().mockRejectedValue(new TypeError("fixture network failure"));
    const client = makeClient(network);
    await expect(client.refreshToken()).rejects.toMatchObject({ code: "network_error", retryable: true });
    expect(network).toHaveBeenCalledTimes(3);
    expect(client.getStoredTokens()?.refreshToken).toBe("old-refresh");
  });
});


describe("expiry and Retry-After boundaries", () => {
  it("honors an HTTP-date Retry-After", async () => {
    vi.useFakeTimers();
    vi.setSystemTime(new Date("2026-10-23T00:00:00Z"));
    const fetchFn = vi.fn<typeof fetch>()
      .mockResolvedValueOnce(response({ error: "temporarily_unavailable" }, 503, new Headers({ "Retry-After": "Fri, 23 Oct 2026 00:00:01 GMT" })))
      .mockResolvedValueOnce(response({ access_token: "fresh", token_type: "Bearer", expires_in: 900 }));
    const pending = makeClient(fetchFn).refreshToken();
    await vi.advanceTimersByTimeAsync(999);
    expect(fetchFn).toHaveBeenCalledTimes(1);
    await vi.advanceTimersByTimeAsync(1);
    await expect(pending).resolves.toMatchObject({ accessToken: "fresh" });
    expect(fetchFn).toHaveBeenCalledTimes(2);
  });

  it("signals login instead of returning a known expired token without refresh authority", async () => {
    const key = "nyxid:tokens:client-1";
    const values = new Map([[key, JSON.stringify({ accessToken: "expired", tokenType: "Bearer", expiresIn: 1, expiresAt: Date.now()-1 })]]);
    const client = makeClient(vi.fn<typeof fetch>(), false, {
      getItem: (name) => values.get(name) ?? null,
      setItem: (name, value) => { values.set(name, value); },
      removeItem: (name) => { values.delete(name); },
    });
    await expect(client.getAccessToken()).rejects.toMatchObject({ code: "missing_refresh_token", retryable: false });
  });
});
