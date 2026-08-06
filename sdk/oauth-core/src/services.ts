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

export type ServiceQueryValue = string | number | boolean | null | undefined;

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

export type TriggerStatus = "active" | "disabled";

export type TriggerVerification =
  | {
      readonly mode: "token";
      readonly location: "bearer" | "query";
    }
  | {
      readonly mode: "hmac_sha256";
      readonly header_name: string;
    };

export type TriggerDelivery =
  | {
      readonly type: "webhook";
      readonly url: string;
    }
  | {
      readonly type: "agent";
      readonly conversation_id: string;
    }
  | {
      readonly type: "notification";
    };

export interface TriggerResponse {
  readonly id: string;
  readonly user_id: string;
  readonly label: string;
  readonly user_service_id: string | null;
  readonly status: TriggerStatus;
  readonly verification: TriggerVerification;
  readonly delivery: TriggerDelivery;
  readonly delivery_signing_key_id: string | null;
  readonly inbound_url: string;
  readonly created_at: string;
  readonly updated_at: string;
}

export interface CreateTriggerInput {
  readonly label: string;
  readonly userServiceId?: string;
  readonly verification: TriggerVerification;
  readonly delivery: TriggerDelivery;
  readonly targetOrgId?: string;
}

export interface UpdateTriggerInput {
  readonly label?: string;
  readonly status?: TriggerStatus;
  readonly delivery?: TriggerDelivery;
}

export interface CreateTriggerResponse {
  readonly trigger: TriggerResponse;
  readonly secret: string;
  readonly delivery_signing_secret: string | null;
  readonly delivery_signing_key_id: string | null;
}

export interface UpdateTriggerResponse {
  readonly trigger: TriggerResponse;
  readonly delivery_signing_secret: string | null;
  readonly delivery_signing_key_id: string | null;
}

export interface ListTriggersResponse {
  readonly triggers: readonly TriggerResponse[];
}

export interface RotateTriggerSecretResponse {
  readonly trigger: TriggerResponse;
  readonly secret: string;
}

export interface RotateTriggerDeliverySecretResponse {
  readonly trigger: TriggerResponse;
  readonly delivery_signing_secret: string;
  readonly key_id: string;
}

export type TriggerDeliveryRecordStatus = "pending" | "delivered" | "failed";

export interface TriggerDeliveryRecord {
  readonly event_id: string;
  readonly status: TriggerDeliveryRecordStatus;
  readonly attempts: number;
  readonly last_status_code: number | null;
  readonly replay_available: boolean;
  readonly created_at: string;
  readonly updated_at: string;
  readonly delivered_at: string | null;
}

export interface ListTriggerDeliveriesOptions {
  readonly page?: number;
  readonly perPage?: number;
}

export interface ListTriggerDeliveriesResponse {
  readonly deliveries: readonly TriggerDeliveryRecord[];
  readonly page: number;
  readonly per_page: number;
  readonly total: number;
}

export interface RedeliverTriggerResponse {
  readonly delivery: TriggerDeliveryRecord;
}

export interface DeleteTriggerResponse {
  readonly message: string;
}

export interface TriggersApi {
  /**
   * Creates an inbound trigger and returns its secret once. The public
   * `inbound_url` is intended for the provider's server-to-server webhook,
   * not for authenticated SDK requests.
   */
  create(input: CreateTriggerInput): Promise<CreateTriggerResponse>;
  list(): Promise<ListTriggersResponse>;
  get(id: string): Promise<TriggerResponse>;
  update(id: string, input: UpdateTriggerInput): Promise<UpdateTriggerResponse>;
  delete(id: string): Promise<DeleteTriggerResponse>;
  rotateSecret(id: string): Promise<RotateTriggerSecretResponse>;
  rotateDeliverySecret(id: string): Promise<RotateTriggerDeliverySecretResponse>;
  listDeliveries(
    id: string,
    options?: ListTriggerDeliveriesOptions,
  ): Promise<ListTriggerDeliveriesResponse>;
  redeliver(id: string, eventId: string): Promise<RedeliverTriggerResponse>;
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
      const message =
        extractErrorMessage(body) ??
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

class TriggersResource implements TriggersApi {
  constructor(private readonly transport: Transport) {}

  create(input: CreateTriggerInput): Promise<CreateTriggerResponse> {
    return this.transport.json<CreateTriggerResponse>("/api/v1/triggers", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({
        label: input.label,
        user_service_id: input.userServiceId,
        verification: input.verification,
        delivery: input.delivery,
        target_org_id: input.targetOrgId,
      }),
    });
  }

  list(): Promise<ListTriggersResponse> {
    return this.transport.json<ListTriggersResponse>("/api/v1/triggers");
  }

  get(id: string): Promise<TriggerResponse> {
    return this.transport.json<TriggerResponse>(
      `/api/v1/triggers/${encodeURIComponent(id)}`,
    );
  }

  update(
    id: string,
    input: UpdateTriggerInput,
  ): Promise<UpdateTriggerResponse> {
    return this.transport.json<UpdateTriggerResponse>(
      `/api/v1/triggers/${encodeURIComponent(id)}`,
      {
        method: "PATCH",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify(input),
      },
    );
  }

  delete(id: string): Promise<DeleteTriggerResponse> {
    return this.transport.json<DeleteTriggerResponse>(
      `/api/v1/triggers/${encodeURIComponent(id)}`,
      { method: "DELETE" },
    );
  }

  rotateSecret(id: string): Promise<RotateTriggerSecretResponse> {
    return this.transport.json<RotateTriggerSecretResponse>(
      `/api/v1/triggers/${encodeURIComponent(id)}/rotate-secret`,
      { method: "POST" },
    );
  }

  rotateDeliverySecret(
    id: string,
  ): Promise<RotateTriggerDeliverySecretResponse> {
    return this.transport.json<RotateTriggerDeliverySecretResponse>(
      `/api/v1/triggers/${encodeURIComponent(id)}/rotate-delivery-secret`,
      { method: "POST" },
    );
  }

  listDeliveries(
    id: string,
    options: ListTriggerDeliveriesOptions = {},
  ): Promise<ListTriggerDeliveriesResponse> {
    const query = new URLSearchParams();
    if (options.page !== undefined) query.set("page", String(options.page));
    if (options.perPage !== undefined) {
      query.set("per_page", String(options.perPage));
    }
    const encodedQuery = query.toString();
    const suffix = encodedQuery ? `?${encodedQuery}` : "";
    return this.transport.json<ListTriggerDeliveriesResponse>(
      `/api/v1/triggers/${encodeURIComponent(id)}/deliveries${suffix}`,
    );
  }

  redeliver(id: string, eventId: string): Promise<RedeliverTriggerResponse> {
    return this.transport.json<RedeliverTriggerResponse>(
      `/api/v1/triggers/${encodeURIComponent(id)}/deliveries/${encodeURIComponent(eventId)}/redeliver`,
      { method: "POST" },
    );
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
  readonly triggers: TriggersApi;

  constructor(config: NyxServicesClientConfig) {
    const transport = new Transport(config);
    this.connectLinks = new ConnectLinksResource(transport);
    this.services = new ServicesResource(transport);
    this.triggers = new TriggersResource(transport);
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
