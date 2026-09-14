import { NyxAppConnectError } from "./requirements.js";
import { NyxServicesHttpError } from "./services.js";

export interface CreateAppConnectLinkOptions {
  readonly callbackUrl: string;
  /** Generate and persist a fresh correlation value before opening the link. */
  readonly state: string;
}

export interface CreatedAppConnectLink {
  readonly id: string;
  readonly connect_url: string;
  readonly expires_at: string;
}

export type AppConnectLinkStatus =
  | "in_progress"
  | "ready_for_consent"
  | "completed"
  | "cancelled"
  | "expired"
  | "failed";

export interface AppConnectLinkItem {
  readonly requirement_id: string;
  readonly label: string;
  readonly optional: boolean;
  readonly state:
    | "unmet"
    | "connecting"
    | "reauthorizing"
    | "validating"
    | "met"
    | "unknown"
    | "failed"
    | "skipped";
  readonly readiness: string;
  readonly user_service_id: string | null;
  readonly slug: string | null;
  readonly resource_uri: string | null;
  readonly owner_id: string | null;
  readonly reason_code: string | null;
  readonly claim: string | null;
  readonly validated_at: string | null;
  readonly valid_until: string | null;
  readonly granted_to_caller: boolean;
}

export interface AppConnectLink {
  readonly id: string;
  readonly oauth_client_id: string;
  readonly client_name: string;
  readonly handoff_blurb: string | null;
  readonly destination: string;
  readonly requirements_version: number;
  readonly status: AppConnectLinkStatus;
  readonly expires_at: string;
  readonly items: readonly AppConnectLinkItem[];
  readonly callback_url: string | null;
  readonly grant_update_required: boolean;
}

export interface AppConnectLinkCallback {
  readonly appConnectLinkId: string;
  readonly state: string;
  readonly status: "completed" | "cancelled" | "expired" | "failed";
  readonly grantUpdateRequired: boolean;
}

/** Parse repair outcomes only after application state correlation succeeds. */
export function parseAppConnectLinkCallback(
  callbackUrl: string,
  expectedState: string,
): AppConnectLinkCallback {
  const params = new URL(callbackUrl).searchParams;
  const state = params.get("state");
  if (
    !expectedState ||
    !state ||
    params.getAll("state").length !== 1 ||
    state !== expectedState
  ) {
    throw new Error("App Connect Link state mismatch");
  }
  if (params.has("error")) throw new NyxAppConnectError(params);
  const status = params.get("status");
  const id = params.get("app_connect_link_id");
  const grant = params.get("grant_update_required");
  if (
    params.getAll("status").length !== 1 ||
    !["completed", "cancelled", "expired", "failed"].includes(status ?? "") ||
    !id ||
    params.getAll("app_connect_link_id").length !== 1 ||
    params.getAll("grant_update_required").length !== 1 ||
    (grant !== "true" && grant !== "false")
  )
    throw new Error("Invalid App Connect Link callback");
  return {
    appConnectLinkId: id,
    state,
    status: status as AppConnectLinkCallback["status"],
    grantUpdateRequired: grant === "true",
  };
}

/** Repair manages connections; it never exchanges a callback for tokens. */
export class NyxAppConnectLinksClient {
  constructor(
    private readonly baseUrl: string,
    private readonly accessToken: () => string | undefined,
    private readonly fetchFn: typeof fetch,
  ) {}

  async create(
    options: CreateAppConnectLinkOptions,
  ): Promise<CreatedAppConnectLink> {
    if (!options.state) throw new Error("App Connect Link state is required");
    return this.request("", "POST", {
      callback_url: options.callbackUrl,
      state: options.state,
    });
  }

  async get(id: string): Promise<AppConnectLink> {
    return this.request(`/${encodeURIComponent(id)}`, "GET");
  }

  parseCallback(
    callbackUrl: string,
    expectedState: string,
  ): AppConnectLinkCallback {
    return parseAppConnectLinkCallback(callbackUrl, expectedState);
  }

  private async request<T>(
    path: string,
    method: string,
    body?: unknown,
  ): Promise<T> {
    const token = this.accessToken();
    if (!token) throw new Error("Missing access token");
    const response = await this.fetchFn(
      `${this.baseUrl}/api/v1/app-connect-links${path}`,
      {
        method,
        headers: {
          Authorization: `Bearer ${token}`,
          ...(body ? { "Content-Type": "application/json" } : {}),
        },
        ...(body ? { body: JSON.stringify(body) } : {}),
      },
    );
    const result: unknown = await response.json();
    if (!response.ok)
      throw new NyxServicesHttpError(
        response.status,
        "App Connect Link request failed",
        result,
      );
    return result as T;
  }
}
