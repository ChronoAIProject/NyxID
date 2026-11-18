export interface NyxIDClientConfig {
  readonly baseUrl: string;
  readonly clientId: string;
  readonly redirectUri: string;
  readonly scope?: string;
  readonly storage?: StorageLike;
  readonly fetchFn?: typeof fetch;
}

export * from "./services.js";
export * from "./webhooks.js";

export interface LoginRedirectOptions {
  readonly scope?: string;
  readonly redirectUri?: string;
  readonly state?: string;
  readonly prompt?: "none" | "consent" | "login" | (string & {});
  readonly includeGrantedScopes?: boolean;
  readonly requestedServiceIds?: readonly string[];
}

export interface NyxIDTokenSet {
  readonly accessToken: string;
  readonly tokenType: string;
  readonly expiresIn: number;
  readonly refreshToken?: string;
  readonly idToken?: string;
  readonly scope?: string;
  /** Epoch milliseconds at which the access token expires. */
  readonly expiresAt?: number;
}

export class NyxIDRefreshError extends Error {
  readonly code: string;
  readonly retryable: boolean;
  readonly retryAfterMs?: number;

  constructor(code: string, message: string, retryable: boolean, retryAfterMs?: number) {
    super(message);
    this.name = "NyxIDRefreshError";
    this.code = code;
    this.retryable = retryable;
    this.retryAfterMs = retryAfterMs;
  }
}

export interface OAuthUserInfo {
  readonly sub: string;
  readonly email?: string;
  readonly email_verified?: boolean;
  readonly name?: string;
  readonly picture?: string;
  readonly roles?: string[];
  readonly groups?: string[];
  readonly permissions?: string[];
}

interface PendingAuthState {
  readonly state: string;
  readonly codeVerifier: string;
  readonly redirectUri: string;
  readonly scope?: string;
}

interface TokenResponse {
  readonly access_token: string;
  readonly token_type: string;
  readonly expires_in: number;
  readonly refresh_token?: string;
  readonly id_token?: string;
  readonly scope?: string;
}

const ACCESS_TOKEN_REFRESH_SKEW_MS = 60_000;
const MAX_REFRESH_ATTEMPTS = 3;
const refreshFlights = new WeakMap<StorageLike, Map<string, Promise<NyxIDTokenSet>>>();

export interface StorageLike {
  getItem(key: string): string | null;
  setItem(key: string, value: string): void;
  removeItem(key: string): void;
}

class MemoryStorage implements StorageLike {
  private readonly data = new Map<string, string>();

  getItem(key: string): string | null {
    return this.data.get(key) ?? null;
  }

  setItem(key: string, value: string): void {
    this.data.set(key, value);
  }

  removeItem(key: string): void {
    this.data.delete(key);
  }
}

function resolveStorage(explicit?: StorageLike): StorageLike {
  if (explicit) return explicit;
  try {
    if (typeof window !== "undefined" && window.localStorage) {
      return window.localStorage;
    }
  } catch {
    // Sandboxed and privacy-mode documents can throw while exposing the
    // localStorage getter. The in-memory store still supports the OAuth flow.
  }
  return new MemoryStorage();
}

function base64UrlEncode(input: Uint8Array): string {
  let binary = "";
  for (const byte of input) binary += String.fromCharCode(byte);
  return btoa(binary)
    .replace(/\+/g, "-")
    .replace(/\//g, "_")
    .replace(/=+$/g, "");
}

function randomUrlSafeString(bytes = 32): string {
  const data = new Uint8Array(bytes);
  crypto.getRandomValues(data);
  return base64UrlEncode(data);
}

async function sha256Base64Url(input: string): Promise<string> {
  const digest = await crypto.subtle.digest(
    "SHA-256",
    new TextEncoder().encode(input),
  );
  return base64UrlEncode(new Uint8Array(digest));
}

function normalizeBaseUrl(baseUrl: string): string {
  return baseUrl.replace(/\/+$/g, "");
}

export class NyxIDClient {
  private readonly baseUrl: string;
  private readonly clientId: string;
  private readonly defaultRedirectUri: string;
  private readonly defaultScope: string;
  private readonly storage: StorageLike;
  private readonly fetchFn: typeof fetch;
  private readonly pendingKey: string;
  private readonly tokensKey: string;
  private refreshPromise: Promise<NyxIDTokenSet> | null = null;

  constructor(config: NyxIDClientConfig) {
    this.baseUrl = normalizeBaseUrl(config.baseUrl);
    this.clientId = config.clientId;
    this.defaultRedirectUri = config.redirectUri;
    this.defaultScope = config.scope ?? "openid profile email";
    this.storage = resolveStorage(config.storage);
    this.fetchFn = config.fetchFn ?? globalThis.fetch.bind(globalThis);
    this.pendingKey = `nyxid:pending:${this.clientId}`;
    this.tokensKey = `nyxid:tokens:${this.clientId}`;
  }

  async buildAuthorizeUrl(options: LoginRedirectOptions = {}): Promise<string> {
    if (options.requestedServiceIds?.length && !options.includeGrantedScopes) {
      throw new Error("requestedServiceIds requires includeGrantedScopes");
    }
    const codeVerifier = randomUrlSafeString(48);
    const codeChallenge = await sha256Base64Url(codeVerifier);
    const state = options.state ?? randomUrlSafeString(24);
    const redirectUri = options.redirectUri ?? this.defaultRedirectUri;
    const scope =
      options.scope ??
      (options.includeGrantedScopes ? undefined : this.defaultScope);

    const pending: PendingAuthState = {
      state,
      codeVerifier,
      redirectUri,
      scope,
    };
    this.storage.setItem(this.pendingKey, JSON.stringify(pending));

    const url = new URL(`${this.baseUrl}/oauth/authorize`);
    url.searchParams.set("response_type", "code");
    url.searchParams.set("client_id", this.clientId);
    url.searchParams.set("redirect_uri", redirectUri);
    if (scope) url.searchParams.set("scope", scope);
    url.searchParams.set("code_challenge", codeChallenge);
    url.searchParams.set("code_challenge_method", "S256");
    url.searchParams.set("state", state);
    if (options.prompt) {
      url.searchParams.set("prompt", options.prompt);
    }
    if (options.includeGrantedScopes) {
      url.searchParams.set("include_granted_scopes", "true");
    }
    for (const id of options.requestedServiceIds ?? []) {
      url.searchParams.append("requested_service_ids", id);
    }
    return url.toString();
  }

  async loginWithRedirect(options: LoginRedirectOptions = {}): Promise<void> {
    if (typeof window === "undefined") {
      throw new Error("loginWithRedirect requires browser environment");
    }
    const url = await this.buildAuthorizeUrl(options);
    window.location.assign(url);
  }

  async handleRedirectCallback(
    currentUrl = window.location.href,
  ): Promise<NyxIDTokenSet> {
    const callback = new URL(currentUrl);
    const oauthError = callback.searchParams.get("error");
    const code = callback.searchParams.get("code");
    const state = callback.searchParams.get("state");
    if (!state) {
      throw new Error("Missing authorization code or state");
    }

    const rawPending = this.storage.getItem(this.pendingKey);
    if (!rawPending) {
      throw new Error("Missing PKCE state in storage");
    }

    let pending: PendingAuthState;
    try {
      pending = JSON.parse(rawPending) as PendingAuthState;
    } catch {
      throw new Error("Invalid PKCE state");
    }

    if (pending.state !== state) {
      throw new Error("State mismatch");
    }

    if (oauthError) {
      this.storage.removeItem(this.pendingKey);
      throw new Error(
        callback.searchParams.get("error_description") ??
          `OAuth error: ${oauthError}`,
      );
    }
    if (!code) {
      throw new Error("Missing authorization code or state");
    }

    const form = new URLSearchParams();
    form.set("grant_type", "authorization_code");
    form.set("code", code);
    form.set("redirect_uri", pending.redirectUri);
    form.set("client_id", this.clientId);
    form.set("code_verifier", pending.codeVerifier);

    const response = await this.fetchFn(`${this.baseUrl}/oauth/token`, {
      method: "POST",
      headers: { "Content-Type": "application/x-www-form-urlencoded" },
      body: form.toString(),
    });

    if (!response.ok) {
      this.storage.removeItem(this.pendingKey);
      const errBody = (await response.json().catch(() => null)) as {
        error?: string;
        error_description?: string;
      } | null;
      const detail =
        errBody?.error_description ?? errBody?.error ?? response.statusText;
      throw new Error(`Token exchange failed: ${detail}`);
    }

    const body = (await response.json()) as TokenResponse;
    const tokens: NyxIDTokenSet = {
      accessToken: body.access_token,
      tokenType: body.token_type,
      expiresIn: body.expires_in,
      refreshToken: body.refresh_token,
      idToken: body.id_token,
      scope: body.scope,
      expiresAt: Date.now() + body.expires_in * 1000,
    };
    this.storage.setItem(this.tokensKey, JSON.stringify(tokens));
    this.storage.removeItem(this.pendingKey);
    return tokens;
  }

  getStoredTokens(): NyxIDTokenSet | null {
    const raw = this.storage.getItem(this.tokensKey);
    if (!raw) return null;
    try {
      return JSON.parse(raw) as NyxIDTokenSet;
    } catch {
      return null;
    }
  }

  /** Refresh the current token set, single-flight within this tab. */
  async refreshToken(): Promise<NyxIDTokenSet> {
    if (this.refreshPromise) return this.refreshPromise;
    let flights = refreshFlights.get(this.storage);
    if (!flights) {
      flights = new Map();
      refreshFlights.set(this.storage, flights);
    }
    const sharedFlight = flights.get(this.tokensKey);
    if (sharedFlight) return sharedFlight;
    const current = this.getStoredTokens();
    if (!current?.refreshToken) {
      throw new NyxIDRefreshError("missing_refresh_token", "Missing refresh token", false);
    }
    this.refreshPromise = this.withRefreshLock(() => this.performRefresh(current))
      .finally(() => {
        this.refreshPromise = null;
        flights.delete(this.tokensKey);
      });
    flights.set(this.tokensKey, this.refreshPromise);
    return this.refreshPromise;
  }

  /** Return a usable access token, refreshing shortly before expiry. */
  async getAccessToken(): Promise<string> {
    const current = this.getStoredTokens();
    if (!current?.accessToken) throw new Error("Missing access token");
    if (
      current.expiresAt !== undefined &&
      current.expiresAt - Date.now() > ACCESS_TOKEN_REFRESH_SKEW_MS
    ) {
      return current.accessToken;
    }
    if (!current.refreshToken) {
      if (current.expiresAt !== undefined && current.expiresAt <= Date.now()) {
        throw new NyxIDRefreshError("missing_refresh_token", "Access token expired; sign in again", false);
      }
      return current.accessToken;
    }
    return (await this.refreshToken()).accessToken;
  }

  private async performRefresh(previous: NyxIDTokenSet): Promise<NyxIDTokenSet> {
    // Another tab may have rotated the token while this tab waited for the
    // lock. Adopt its successor without sending the old refresh token again.
    const latest = this.getStoredTokens();
    if (!latest) throw new NyxIDRefreshError("session_changed", "Session was cleared during refresh", false);
    if (latest.refreshToken !== previous.refreshToken) return latest;
    const form = new URLSearchParams({
      grant_type: "refresh_token",
      refresh_token: previous.refreshToken!,
      client_id: this.clientId,
    });
    let lastError: NyxIDRefreshError | null = null;
    for (let attempt = 0; attempt < MAX_REFRESH_ATTEMPTS; attempt += 1) {
      const current = this.getStoredTokens();
      if (!current) throw new NyxIDRefreshError("session_changed", "Session was cleared during refresh", false);
      if (current.refreshToken !== previous.refreshToken) return current;
      try {
        const response = await this.fetchFn(`${this.baseUrl}/oauth/token`, {
          method: "POST",
          headers: { "Content-Type": "application/x-www-form-urlencoded" },
          body: form.toString(),
        });
        if (response.ok) {
          const body = (await response.json()) as TokenResponse;
          const tokens: NyxIDTokenSet = {
            accessToken: body.access_token,
            tokenType: body.token_type,
            expiresIn: body.expires_in,
            refreshToken: body.refresh_token ?? previous.refreshToken,
            idToken: body.id_token,
            scope: body.scope ?? previous.scope,
            expiresAt: Date.now() + body.expires_in * 1000,
          };
          const after = this.getStoredTokens();
          if (!after) throw new NyxIDRefreshError("session_changed", "Session was cleared during refresh", false);
          if (after.refreshToken !== previous.refreshToken) return after;
          // One synchronous write is the atomic storage transition visible to
          // this SDK. Never remove the old token before this succeeds.
          this.storage.setItem(this.tokensKey, JSON.stringify(tokens));
          return tokens;
        }
        const body = (await response.json().catch(() => null)) as { error?: string; error_description?: string } | null;
        if (body?.error === "invalid_grant") {
          const after = this.getStoredTokens();
          if (after && after.refreshToken !== previous.refreshToken) return after;
          this.storage.removeItem(this.tokensKey);
          throw new NyxIDRefreshError("invalid_grant", body.error_description ?? "Refresh token is no longer valid", false);
        }
        const retryable = response.status === 503 || body?.error === "temporarily_unavailable";
        if (!retryable) {
          throw new NyxIDRefreshError(body?.error ?? "refresh_failed", body?.error_description ?? `Token refresh failed (${response.status})`, false);
        }
        const header = response.headers.get("Retry-After");
        const retryAfterMs = header
          ? /^\d+(?:\.\d+)?$/.test(header)
            ? Number(header) * 1000
            : Math.max(0, Date.parse(header) - Date.now()) || undefined
          : undefined;
        lastError = new NyxIDRefreshError(body?.error ?? "temporarily_unavailable", body?.error_description ?? "Token refresh is temporarily unavailable", true, retryAfterMs);
      } catch (error) {
        if (error instanceof NyxIDRefreshError && !error.retryable) throw error;
        lastError = error instanceof NyxIDRefreshError
          ? error
          : new NyxIDRefreshError("network_error", "Token refresh network request failed", true);
      }
      if (attempt + 1 < MAX_REFRESH_ATTEMPTS) {
        await new Promise((resolve) => setTimeout(resolve, lastError?.retryAfterMs ?? 250 * 2 ** attempt));
      }
    }
    throw lastError ?? new NyxIDRefreshError("temporarily_unavailable", "Token refresh failed", true);
  }

  private async withRefreshLock<T>(task: () => Promise<T>): Promise<T> {
    const locks = (globalThis as { navigator?: { locks?: { request: <R>(name: string, callback: () => Promise<R>) => Promise<R> } } }).navigator?.locks;
    if (locks) return locks.request(`nyxid:refresh:${this.clientId}`, task);
    // Shared persistent storage requires an actual cross-document mutex.
    // localStorage read/write leases cannot implement CAS and can expire while
    // a background tab is suspended. Fail closed rather than reuse a rotated
    // credential. Tab-local/custom storage retains in-tab single-flight.
    let shared = false;
    try {
      shared = typeof window !== "undefined" && this.storage === window.localStorage;
    } catch {
      // A throwing localStorage getter means the resolved store is tab-local.
    }
    if (shared) {
      throw new NyxIDRefreshError(
        "refresh_coordination_unavailable",
        "Shared token refresh requires Web Locks; use tab-local storage in this browser",
        true,
      );
    }
    return task();
  }

  clearSession(): void {
    this.storage.removeItem(this.pendingKey);
    this.storage.removeItem(this.tokensKey);
  }

  async getUserInfo(accessToken?: string): Promise<OAuthUserInfo> {
    const token = accessToken ?? await this.getAccessToken();
    if (!token) {
      throw new Error("Missing access token");
    }

    const response = await this.fetchFn(`${this.baseUrl}/oauth/userinfo`, {
      method: "GET",
      headers: { Authorization: `Bearer ${token}` },
    });
    if (!response.ok) {
      throw new Error("Failed to fetch userinfo");
    }
    return (await response.json()) as OAuthUserInfo;
  }
}
