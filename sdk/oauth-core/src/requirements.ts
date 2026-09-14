import { NyxServicesHttpError } from "./services.js";

export type AppRequirementState =
  | "unmet"
  | "met"
  | "included"
  | "unknown"
  | "broken"
  | "needs_reauth"
  | "unsatisfiable"
  | "disabled";

export interface AppRequirementStatus {
  readonly requirement_id: string;
  readonly state: AppRequirementState;
  readonly user_service_id: string | null;
  readonly slug: string | null;
  readonly resource_uri: string | null;
  readonly owner_id: string | null;
  readonly validated_at: string | null;
  readonly valid_until: string | null;
  readonly credential_health: string | null;
  readonly granted_to_caller: boolean;
}

export interface AppRequirementsStatus {
  readonly requirements_version: number;
  readonly result_id: string;
  readonly requirements: readonly AppRequirementStatus[];
}

/** Local readiness only. The token's own grant is reported separately. */
export class NyxRequirementsClient {
  constructor(
    private readonly baseUrl: string,
    private readonly accessToken: () => string | undefined,
    private readonly fetchFn: typeof fetch,
  ) {}

  async status(): Promise<AppRequirementsStatus> {
    const token = this.accessToken();
    if (!token) throw new Error("Missing access token");
    const response = await this.fetchFn(
      `${this.baseUrl}/api/v1/app-requirements/status`,
      {
        method: "GET",
        headers: { Authorization: `Bearer ${token}` },
      },
    );
    const body: unknown = await response.json();
    if (!response.ok) {
      throw new NyxServicesHttpError(
        response.status,
        "Failed to read app requirements",
        body,
      );
    }
    return body as AppRequirementsStatus;
  }
}

/** Created only after the pending OAuth state's correlation has succeeded. */
export class NyxAppConnectError extends Error {
  readonly error: string;
  readonly status: string | null;
  readonly reason: string | null;
  readonly appConnectLinkId: string | null;

  constructor(params: URLSearchParams) {
    const error = params.get("error") ?? "invalid_request";
    super(params.get("error_description") ?? `OAuth error: ${error}`);
    this.name = "NyxAppConnectError";
    this.error = error;
    this.status = params.get("nyx_connect_status");
    this.reason = params.get("nyx_connect_reason");
    this.appConnectLinkId = params.get("app_connect_link_id");
  }
}
