import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { ApiError } from "@/lib/api-client";
import {
  routeAgentIssue,
  verificationErrorMessage,
} from "@/lib/channel-readiness";
import type { ApiErrorResponse, ApiKey } from "@/types/api";
import type {
  BotVerification,
  ChannelConversationItem,
} from "@/types/channels";
import { CredentialVerification, RouteReadiness } from "./bot-readiness";

const checked: BotVerification = {
  id: "old",
  status: "verified",
  started_at: "2026-09-20T00:00:00Z",
  completed_at: "2026-09-20T00:00:01Z",
  message: null,
};
const key: ApiKey = {
  id: "agent",
  name: "Org agent",
  description: null,
  key_prefix: "test",
  scopes: "",
  created_at: "2026-09-20T00:00:00Z",
  last_used_at: null,
  expires_at: null,
  is_active: true,
  allowed_service_ids: [],
  allowed_node_ids: [],
  allow_all_services: false,
  allow_all_nodes: false,
  allowed_services: [],
  allowed_nodes: [],
  platform: null,
  callback_url: "https://agent.test/callback",
  rate_limit_per_second: null,
  rate_limit_burst: null,
  bindings_count: 0,
  credential_source: {
    type: "org",
    org_id: "org-1",
    org_name: "ChronoAI",
    role: "admin",
    allowed: true,
  },
};

describe("credential verification observations", () => {
  it.each([
    { message: "edge detail" },
    { error_code: null, message: "edge detail" },
    { error_code: "10005", message: "edge detail" },
    { error_code: 999999, message: "edge detail" },
    { error_code: 10005.5, message: "edge detail" },
    { error_code: 10005 },
    { error_code: 10005, message: { detail: "edge detail" } },
    { error_code: 10005, message: null },
  ])("sanitizes malformed or unknown JSON errors: %j", (body) => {
    const error = new ApiError(502, body as ApiErrorResponse);
    expect(verificationErrorMessage(error)).toContain(
      "No usable verification result",
    );
    expect(verificationErrorMessage(error)).toContain("HTTP 502");
    expect(verificationErrorMessage(error)).not.toContain("edge detail");
  });

  it("preserves useful messages from recognized NyxID errors", () => {
    const message =
      "WhatsApp: access token is invalid or expired (HTTP 400, code 190)";
    expect(
      verificationErrorMessage(
        new ApiError(502, {
          error: "channel_platform_error",
          error_code: 10005,
          message,
        }),
      ),
    ).toBe(message);
  });

  it("shows an opaque failure over an unchanged prior success, then accepts a newer server check", () => {
    const error = verificationErrorMessage(
      new ApiError(502, {
        error: "unknown_error",
        error_code: -1,
        message: "Request failed",
      }),
    );
    const view = render(
      <CredentialVerification
        check={checked}
        pending={false}
        error={error}
        previousCheckId="old"
      />,
    );
    expect(screen.getByText("Credential check failed")).toBeInTheDocument();
    expect(screen.queryByText("Credentials verified")).not.toBeInTheDocument();
    expect(screen.getByText(/No usable verification result/)).toHaveTextContent(
      "HTTP 502",
    );
    expect(screen.queryByText(/expired/)).not.toBeInTheDocument();
    view.rerender(
      <CredentialVerification
        check={{ ...checked, id: "new" }}
        pending={false}
        error={error}
        previousCheckId="old"
      />,
    );
    expect(screen.getByText("Credentials verified")).toBeInTheDocument();
    expect(
      screen.queryByText(/No usable verification result/),
    ).not.toBeInTheDocument();
  });

  it("uses the persisted failure and pending/incomplete state instead of historical success", () => {
    const view = render(
      <CredentialVerification
        check={checked}
        pending
        error={null}
        previousCheckId="old"
      />,
    );
    expect(screen.getByText("Checking credentials…")).toBeInTheDocument();
    expect(screen.queryByText("Credentials verified")).not.toBeInTheDocument();
    view.rerender(
      <CredentialVerification
        check={{
          ...checked,
          id: "new",
          status: "failed",
          message: "Access token is invalid or expired",
        }}
        pending={false}
        error="Old transport failure"
        previousCheckId="old"
      />,
    );
    expect(
      screen.getByText("Access token is invalid or expired"),
    ).toBeInTheDocument();
    expect(screen.queryByText("Old transport failure")).not.toBeInTheDocument();
    view.rerender(
      <CredentialVerification
        check={{ ...checked, status: "incomplete" }}
        pending={false}
        error={null}
        previousCheckId="old"
      />,
    );
    expect(screen.getByText("Credential check incomplete")).toBeInTheDocument();
    view.rerender(
      <CredentialVerification
        check={null}
        pending={false}
        error={null}
        previousCheckId="old"
      />,
    );
    expect(screen.getByText("Credentials not checked")).toBeInTheDocument();
  });
});

describe("owner-scoped route setup", () => {
  it("explains an empty organization without suggesting a callback alone registers a runtime", () => {
    render(
      <RouteReadiness
        keys={[]}
        ownerOrgId="org-1"
        ownerLabel="ChronoAI"
        conversations={[]}
      />,
    );
    expect(
      screen.getByText("No agent keys exist in this owner scope."),
    ).toBeInTheDocument();
    expect(
      screen.getByText(/Setting a callback URL alone does not register/),
    ).toHaveTextContent("ChronoAI");
    expect(
      screen.getByRole("link", { name: "Open Agent Keys" }),
    ).toHaveAttribute("href", "/keys?tab=nyxid");
  });

  it("links to the actual same-owner key missing its callback", () => {
    render(
      <RouteReadiness
        keys={[{ ...key, callback_url: null }]}
        ownerOrgId="org-1"
        ownerLabel="ChronoAI"
      />,
    );
    expect(
      screen.getByRole("link", { name: "Configure Org agent" }),
    ).toHaveAttribute("href", "/keys/api-key/agent");
    expect(
      screen.getByText(
        "No eligible agent key is configured in this owner scope.",
      ),
    ).toBeInTheDocument();
  });

  it("does not present a broken route as configured", () => {
    const route = {
      is_active: true,
      agent_api_key_id: "agent",
    } as ChannelConversationItem;
    render(
      <RouteReadiness
        keys={[{ ...key, is_active: false }]}
        ownerOrgId="org-1"
        ownerLabel="ChronoAI"
        conversations={[route]}
      />,
    );
    expect(
      screen.queryByText(/An agent route is configured/),
    ).not.toBeInTheDocument();
  });

  it("uses the same eligibility exclusions for key selection", () => {
    expect(routeAgentIssue(key, "org-1")).toBeNull();
    expect(routeAgentIssue(key, null)).toBe("Different owner");
    expect(routeAgentIssue(key, "org-2")).toBe("Different owner");
    expect(
      routeAgentIssue(
        { ...key, credential_source: { type: "personal" } },
        "org-1",
      ),
    ).toBe("Different owner");
    expect(routeAgentIssue({ ...key, is_active: false }, "org-1")).toBe(
      "Inactive",
    );
    expect(
      routeAgentIssue({ ...key, expires_at: "2020-01-01T00:00:00Z" }, "org-1"),
    ).toBe("Expired");
    expect(
      routeAgentIssue({ ...key, platform: "nyxid-assistant" }, "org-1"),
    ).toBe("Assistant chat key");
    expect(routeAgentIssue({ ...key, callback_url: "  " }, "org-1")).toBe(
      "No callback URL",
    );
  });
});
