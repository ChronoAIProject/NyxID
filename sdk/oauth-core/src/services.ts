export type NyxServicesAuth =
  | { readonly apiKey: string; readonly accessToken?: never }
  | { readonly accessToken: string; readonly apiKey?: never };

export interface NyxServicesClientConfig {
  readonly baseUrl: string;
  readonly auth: NyxServicesAuth;
  readonly fetchFn?: typeof fetch;
}

export type ConnectLinkStatus =
  | "pending"
  | "completed"
  | "expired"
  | "cancelled";

export interface CreateConnectLinkInput {
  readonly serviceSlug: string;
  readonly label?: string;
  /**
   * Browser return URI. For an OAuth access token issued to a registered app,
   * this must match that app's redirect URI policy. Terminal redirects add
   * `status` and `connect_link_id` query parameters and never include the raw
   * connect token.
   */
  readonly callbackUrl?: string;
  readonly expiresIn?: number;
}

export interface CreateConnectLinkResponse {
  readonly id: string;
  readonly connect_url: string;
  readonly expires_at: string;
}

export interface ConnectedService {
  readonly id: string;
  readonly slug: string;
}

export interface ConnectLinkResponse {
  readonly id: string;
  readonly status: ConnectLinkStatus;
  readonly service_name: string;
  readonly service_slug: string;
  readonly expires_at: string;
  readonly completed_at?: string;
  readonly connected_service?: ConnectedService;
  /** Present only for terminal outcomes, with status and link-id query params. */
  readonly callback_url?: string;
  readonly requesting_app_id?: string;
  readonly requesting_app_name?: string;
  /** Stable provider-decline code while the link remains retryable. */
  readonly last_error?: string;
  readonly last_error_at?: string;
}

export interface WaitForCompletionOptions {
  readonly timeoutMs?: number;
  readonly intervalMs?: number;
}

export type ServiceQueryValue =
  | string
  | number
  | boolean
  | null
  | undefined;

export interface ServiceRequestOptions {
  readonly method?: string;
  readonly headers?: HeadersInit;
  readonly body?: BodyInit | null;
  readonly query?: Readonly<
    Record<string, ServiceQueryValue | readonly ServiceQueryValue[]>
  >;
}

export class NyxServicesHttpError extends Error {
  readonly status: number;
  readonly body: unknown;

  constructor(status: number, message: string, body: unknown) {
    super(message);
    this.name = "NyxServicesHttpError";
    this.status = status;
    this.body = body;
  }
}

export class ConnectLinkExpiredError extends Error {
  readonly connectLinkId: string;

  constructor(connectLinkId: string) {
    super(`Connect link ${connectLinkId} expired`);
    this.name = "ConnectLinkExpiredError";
    this.connectLinkId = connectLinkId;
  }
}

export class ConnectLinkCancelledError extends Error {
  readonly connectLinkId: string;

  constructor(connectLinkId: string) {
    super(`Connect link ${connectLinkId} was cancelled`);
    this.name = "ConnectLinkCancelledError";
    this.connectLinkId = connectLinkId;
  }
}

export class ConnectLinkTimeoutError extends Error {
  readonly connectLinkId: string;
  readonly timeoutMs: number;

  constructor(connectLinkId: string, timeoutMs: number) {
    super(`Timed out waiting ${timeoutMs}ms for connect link ${connectLinkId}`);
    this.name = "ConnectLinkTimeoutError";
    this.connectLinkId = connectLinkId;
    this.timeoutMs = timeoutMs;
  }
}

export interface ConnectLinksApi {
  /**
   * Creates a browser connection flow. Credential submission and OAuth
   * completion remain human-only browser operations; this SDK intentionally
   * does not expose the completion endpoint. A terminal callback contains
   * `status=completed|cancelled|expired` and `connect_link_id`, never the raw
   * connect token.
   */
  create(input: CreateConnectLinkInput): Promise<CreateConnectLinkResponse>;
  get(id: string): Promise<ConnectLinkResponse>;
  waitForCompletion(
    id: string,
    options?: WaitForCompletionOptions,
  ): Promise<ConnectedService>;
  cancel(id: string): Promise<ConnectLinkResponse>;
}

export interface ServicesApi {
  /** Sends an authenticated request through a connected service. */
  request(
    slug: string,
    path: string,
    options?: ServiceRequestOptions,
  ): Promise<Response>;
}

class Transport {
  readonly baseUrl: string;
  readonly fetchFn: typeof fetch;
  readonly #authorization: string;

  constructor(config: NyxServicesClientConfig) {
    this.baseUrl = trimTrailingSlashes(config.baseUrl);
    this.fetchFn = config.fetchFn ?? globalThis.fetch.bind(globalThis);
    const credential = config.auth.apiKey ?? config.auth.accessToken;
    if (credential === undefined || !credential.trim()) {
      throw new Error("NyxServicesClient auth credential must not be empty");
    }
    this.#authorization = `Bearer ${credential}`;
  }

  async json<T>(path: string, init: RequestInit = {}): Promise<T> {
    const headers = this.authenticatedHeaders(init.headers);
    const response = await this.fetchFn(`${this.baseUrl}${path}`, {
      ...init,
      headers,
    });
    if (!response.ok) {
      const body: unknown = await response.json().catch(() => null);
      const message = extractErrorMessage(body) ??
        `NyxID request failed with HTTP ${response.status}`;
      throw new NyxServicesHttpError(response.status, message, body);
    }
    return (await response.json()) as T;
  }

  authenticatedHeaders(initial?: HeadersInit): Headers {
    const headers = new Headers(initial);
    headers.set("Authorization", this.#authorization);
    return headers;
  }
}

class ConnectLinksResource implements ConnectLinksApi {
  constructor(private readonly transport: Transport) {}

  create(input: CreateConnectLinkInput): Promise<CreateConnectLinkResponse> {
    return this.transport.json<CreateConnectLinkResponse>(
      "/api/v1/connect-links",
      {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({
          service_slug: input.serviceSlug,
          label: input.label,
          callback_url: input.callbackUrl,
          expires_in: input.expiresIn,
        }),
      },
    );
  }

  get(id: string): Promise<ConnectLinkResponse> {
    return this.transport.json<ConnectLinkResponse>(
      `/api/v1/connect-links/${encodeURIComponent(id)}`,
    );
  }

  async waitForCompletion(
    id: string,
    options: WaitForCompletionOptions = {},
  ): Promise<ConnectedService> {
    const timeoutMs = options.timeoutMs ?? 120_000;
    const intervalMs = options.intervalMs ?? 1_000;
    if (!Number.isFinite(timeoutMs) || timeoutMs < 0) {
      throw new RangeError("timeoutMs must be a finite non-negative number");
    }
    if (!Number.isFinite(intervalMs) || intervalMs <= 0) {
      throw new RangeError("intervalMs must be a finite positive number");
    }

    const deadline = Date.now() + timeoutMs;
    while (Date.now() <= deadline) {
      const link = await this.get(id);
      if (link.status === "completed") {
        if (!link.connected_service) {
          throw new Error(
            `Completed connect link ${id} did not include connected_service`,
          );
        }
        return link.connected_service;
      }
      if (link.status === "expired") {
        throw new ConnectLinkExpiredError(id);
      }
      if (link.status === "cancelled") {
        throw new ConnectLinkCancelledError(id);
      }

      const remaining = deadline - Date.now();
      if (remaining <= 0) break;
      await delay(Math.min(intervalMs, remaining));
    }
    throw new ConnectLinkTimeoutError(id, timeoutMs);
  }

  cancel(id: string): Promise<ConnectLinkResponse> {
    return this.transport.json<ConnectLinkResponse>(
      `/api/v1/connect-links/${encodeURIComponent(id)}/cancel`,
      { method: "POST" },
    );
  }
}

class ServicesResource implements ServicesApi {
  constructor(private readonly transport: Transport) {}

  request(
    slug: string,
    path: string,
    options: ServiceRequestOptions = {},
  ): Promise<Response> {
    const normalizedPath = trimLeadingSlashes(path);
    const url = new URL(
      `${this.transport.baseUrl}/api/v1/proxy/s/${encodeURIComponent(slug)}/${normalizedPath}`,
    );
    appendQuery(url.searchParams, options.query);

    const headers = this.transport.authenticatedHeaders(options.headers);
    return this.transport.fetchFn(url, {
      method: options.method ?? "GET",
      headers,
      body: options.body,
    });
  }
}

/**
 * Authenticated client for connect-link orchestration and service proxy calls.
 * It accepts either a NyxID agent API key or an OAuth access token without
 * modifying the process-wide fetch implementation.
 */
export class NyxServicesClient {
  readonly connectLinks: ConnectLinksApi;
  readonly services: ServicesApi;

  constructor(config: NyxServicesClientConfig) {
    const transport = new Transport(config);
    this.connectLinks = new ConnectLinksResource(transport);
    this.services = new ServicesResource(transport);
  }
}

function appendQuery(
  searchParams: URLSearchParams,
  query: ServiceRequestOptions["query"],
): void {
  if (!query) return;
  for (const [key, rawValue] of Object.entries(query)) {
    const values = Array.isArray(rawValue) ? rawValue : [rawValue];
    for (const value of values) {
      if (value !== null && value !== undefined) {
        searchParams.append(key, String(value));
      }
    }
  }
}

function extractErrorMessage(body: unknown): string | undefined {
  if (!body || typeof body !== "object") return undefined;
  const record = body as Record<string, unknown>;
  if (typeof record.message === "string") return record.message;
  if (typeof record.error === "string") return record.error;
  return undefined;
}

function trimTrailingSlashes(value: string): string {
  let end = value.length;
  while (end > 0 && value.charCodeAt(end - 1) === 0x2f) {
    end -= 1;
  }
  return value.slice(0, end);
}

function trimLeadingSlashes(value: string): string {
  let start = 0;
  while (start < value.length && value.charCodeAt(start) === 0x2f) {
    start += 1;
  }
  return value.slice(start);
}

function delay(durationMs: number): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, durationMs));
}
