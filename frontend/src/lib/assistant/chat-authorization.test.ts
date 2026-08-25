import { describe, expect, it } from "vitest";
import {
  authorizationBlockerToConnectCard,
  parseAuthorizationBlocker,
  parseToolResultBlocker,
} from "./chat-authorization";

describe("chat authorization blockers", () => {
  it("accepts only the typed NyxID reason codes and redacts the safe message", () => {
    const blocker = parseAuthorizationBlocker({
      serviceSlug: "api-github",
      serviceLabel: "GitHub",
      reasonCode: "NYXID_UNAUTHORIZED",
      safeMessage: "Connect GitHub; token=secret-value is expired.",
      resourceUri: "/repos/private?access_token=do-not-render",
    });
    expect(blocker).toEqual({
      serviceSlug: "api-github",
      serviceLabel: "GitHub",
      reasonCode: "NYXID_UNAUTHORIZED",
      safeMessage: 'Connect GitHub; token="[redacted]" is expired.',
    });
    expect(authorizationBlockerToConnectCard(blocker!)).toMatchObject({
      catalog_slug: "api-github",
      reason_code: "NYXID_UNAUTHORIZED",
      state: "needs_connection",
    });
    expect(
      parseAuthorizationBlocker({
        serviceSlug: "api-github",
        reasonCode: "POLICY_DENIED",
      }),
    ).toBeNull();
  });

  it("recognizes the complete readiness DTO carried by TOOL_CALL_END", () => {
    expect(
      parseToolResultBlocker(
        JSON.stringify({
          blocked: true,
          service_slug: "api-github",
          readiness_status: "ServiceRegistrationRequired",
          reason_code: "USER_SERVICE_NOT_VISIBLE",
          safe_message: "No visible service.",
        }),
      ),
    ).toEqual({
      reasonCode: "NYXID_SERVICE_NOT_CONNECTED",
      safeMessage: "Connect Github to continue.",
      serviceLabel: "Github",
      serviceSlug: "api-github",
    });
    expect(
      parseToolResultBlocker(
        JSON.stringify({ blocked: true, service_slug: "api-github" }),
      ),
    ).toBeNull();
  });
});
