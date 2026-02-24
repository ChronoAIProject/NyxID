export interface NyxIDClientConfig {
  readonly baseUrl: string;
  readonly clientId: string;
  readonly redirectUri: string;
  readonly scope?: string;
  readonly storage?: StorageLike;
  readonly fetchFn?: typeof fetch;
}

export interface LoginRedirectOptions {
  readonly scope?: string;
  readonly redirectUri?: string;
  readonly state?: string;
  readonly prompt?: "none" | "consent" | "login" | (string & {});
}

export interface NyxIDTokenSet {
  readonly accessToken: string;
  readonly tokenType: string;
  readonly expiresIn: number;
  readonly refreshToken?: string;
  readonly idToken?: string;
  readonly scope?: string;
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
  readonly scope: string;
}

interface TokenResponse {
  readonly access_token: string;
  readonly token_type: string;
  readonly expires_in: number;
  readonly refresh_token?: string;
  readonly id_token?: string;
  readonly scope?: string;
}

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
  if (typeof window !== "undefined" && window.localStorage) {
    return window.localStorage;
  }
  return new MemoryStorage();
}

function base64UrlEncode(input: Uint8Array): string {
  let binary = "";
  for (const byte of input) binary += String.fromCharCode(byte);
  return btoa(binary).replace(/\+/g, "-").replace(/\//g, "_").replace(/=+$/g, "");
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
    const codeVerifier = randomUrlSafeString(48);
    const codeChallenge = await sha256Base64Url(codeVerifier);
    const state = options.state ?? randomUrlSafeString(24);
    const redirectUri = options.redirectUri ?? this.defaultRedirectUri;
    const scope = options.scope ?? this.defaultScope;

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
    url.searchParams.set("scope", scope);
    url.searchParams.set("code_challenge", codeChallenge);
    url.searchParams.set("code_challenge_method", "S256");
    url.searchParams.set("state", state);
    if (options.prompt) {
      url.searchParams.set("prompt", options.prompt);
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

  async handleRedirectCallback(currentUrl = window.location.href): Promise<NyxIDTokenSet> {
    const callback = new URL(currentUrl);
    const oauthError = callback.searchParams.get("error");
    if (oauthError) {
      throw new Error(
        callback.searchParams.get("error_description") ?? `OAuth error: ${oauthError}`,
      );
    }

    const code = callback.searchParams.get("code");
    const state = callback.searchParams.get("state");
    if (!code || !state) {
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
      const errBody = await response.json().catch(() => null) as {
        error?: string;
        error_description?: string;
      } | null;
      const detail = errBody?.error_description ?? errBody?.error ?? response.statusText;
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

  clearSession(): void {
    this.storage.removeItem(this.pendingKey);
    this.storage.removeItem(this.tokensKey);
  }

  async getUserInfo(accessToken?: string): Promise<OAuthUserInfo> {
    const token = accessToken ?? this.getStoredTokens()?.accessToken;
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
