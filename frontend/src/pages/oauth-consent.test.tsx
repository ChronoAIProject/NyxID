import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { vi } from "vitest";

import { OAuthConsentPage } from "./oauth-consent";

const { state } = vi.hoisted(() => ({
  state: {
    userServices: [] as Array<{
      id: string;
      slug: string;
      resource_uri: string;
      auth_method: string;
      is_active: boolean;
      credential_source: { type: "personal" } | { type: "org" };
    }>,
    userServicesLoading: false,
  },
}));

vi.mock("@/hooks/use-user-services", () => ({
  useUserServices: () => ({
    data: state.userServices,
    isLoading: state.userServicesLoading,
  }),
}));

function setSearch(params: Record<string, string | readonly string[]>) {
  const qs = new URLSearchParams();
  for (const [key, value] of Object.entries(params)) {
    if (typeof value === "string") {
      qs.set(key, value);
    } else {
      for (const item of value) qs.append(key, item);
    }
  }
  window.history.pushState({}, "", `/oauth/consent?${qs}`);
}

// A complete, valid set of required params so the page renders the consent UI.
const VALID = {
  response_type: "code",
  client_id: "client-abc",
  redirect_uri: "https://app.example.com/callback",
  scope: "openid profile email offline_access custom:thing",
  code_challenge: "challenge-xyz",
  code_challenge_method: "S256",
  consent_request: "signed-consent-request-token",
  state: "state-123",
  nonce: "nonce-456",
};

function hiddenInput(name: string): HTMLInputElement | null {
  return document.querySelector<HTMLInputElement>(
    `input[type="hidden"][name="${name}"]`,
  );
}

function hiddenInputs(name: string): HTMLInputElement[] {
  return Array.from(
    document.querySelectorAll<HTMLInputElement>(
      `input[type="hidden"][name="${name}"]`,
    ),
  );
}

beforeEach(() => {
  window.history.pushState({}, "", "/");
  state.userServices = [
    {
      id: "svc-openai",
      slug: "openai",
      resource_uri: "https://nyx.example/api/v1/proxy/s/openai",
      auth_method: "bearer",
      is_active: true,
      credential_source: { type: "personal" },
    },
    {
      id: "svc-inactive",
      slug: "inactive",
      resource_uri: "https://nyx.example/api/v1/proxy/s/inactive",
      auth_method: "bearer",
      is_active: false,
      credential_source: { type: "personal" },
    },
    {
      id: "svc-org",
      slug: "org-service",
      resource_uri: "https://nyx.example/api/v1/proxy/s/org-service",
      auth_method: "bearer",
      is_active: true,
      credential_source: { type: "org" },
    },
  ];
  state.userServicesLoading = false;
});

afterEach(() => {
  window.history.pushState({}, "", "/");
});

describe("OAuthConsentPage", () => {
  it("renders the invalid-request card when a required param is missing", () => {
    // Drop code_challenge -> `missing` is true.
    const { code_challenge, ...rest } = VALID;
    void code_challenge;
    setSearch(rest);

    render(<OAuthConsentPage />);

    expect(screen.getByText("Invalid consent request")).toBeInTheDocument();
    // The consent form must not render in the missing branch.
    expect(
      screen.queryByRole("button", { name: "Allow" }),
    ).not.toBeInTheDocument();
  });

  it("renders one scope badge per whitespace-separated scope", () => {
    setSearch(VALID);

    render(<OAuthConsentPage />);

    // "Requested scopes" section: each scope is a Badge.
    for (const scope of VALID.scope.split(" ")) {
      expect(screen.getAllByText(scope).length).toBeGreaterThan(0);
    }
  });

  it("renders the client name and parsed redirect host", () => {
    setSearch({ ...VALID, client_name: "My Cool App" });

    render(<OAuthConsentPage />);

    // clientName falls back to client_id; here it's the explicit client_name.
    expect(screen.getAllByText("My Cool App").length).toBeGreaterThan(0);
    // parseHost("https://app.example.com/callback") === "app.example.com".
    expect(screen.getByText("app.example.com")).toBeInTheDocument();
    // Full client_id and redirect_uri are shown in their detail blocks.
    expect(screen.getByText("client-abc")).toBeInTheDocument();
    expect(
      screen.getByText("https://app.example.com/callback"),
    ).toBeInTheDocument();
  });

  it("falls back to client_id as the display name when client_name is absent", () => {
    setSearch(VALID);

    render(<OAuthConsentPage />);

    // clientName = client_name || clientId => "client-abc" appears as the app name.
    expect(screen.getAllByText("client-abc").length).toBeGreaterThan(0);
  });

  it("maps known scope risk levels and labels unknown scopes as Custom permission/Medium", () => {
    setSearch(VALID);

    render(<OAuthConsentPage />);

    // Risk labels from scopeRiskLabel(): offline_access => High, email => Medium,
    // openid/profile => Low. Each appears in the scope-impact list.
    expect(screen.getAllByText("High").length).toBeGreaterThan(0);
    expect(screen.getAllByText("Medium").length).toBeGreaterThan(0);
    expect(screen.getAllByText("Low").length).toBeGreaterThan(0);

    // Known scope title from OAUTH_SCOPE_META.
    expect(screen.getByText("Long-lived access")).toBeInTheDocument();
    // Unknown scope ("custom:thing") gets the default meta.
    expect(screen.getByText("Custom permission")).toBeInTheDocument();
  });

  it("posts the consent decision form to /oauth/authorize/decision", () => {
    setSearch(VALID);

    render(<OAuthConsentPage />);

    const allow = screen.getByRole("button", { name: "Allow" });
    const form = allow.closest("form")!;
    expect(form.getAttribute("action")).toBe("/oauth/authorize/decision");
    expect(form.getAttribute("method")?.toLowerCase()).toBe("post");
  });

  it("Allow and Deny are submit buttons carrying the decision value", () => {
    setSearch(VALID);

    render(<OAuthConsentPage />);

    const allow = screen.getByRole("button", { name: "Allow" });
    const deny = screen.getByRole("button", { name: "Deny" });

    expect(allow).toHaveAttribute("type", "submit");
    expect(allow).toHaveAttribute("name", "decision");
    expect(allow).toHaveAttribute("value", "allow");

    expect(deny).toHaveAttribute("type", "submit");
    expect(deny).toHaveAttribute("name", "decision");
    expect(deny).toHaveAttribute("value", "deny");
  });

  it("forwards required OAuth params as hidden inputs", () => {
    setSearch(VALID);

    render(<OAuthConsentPage />);

    expect(hiddenInput("response_type")?.value).toBe("code");
    expect(hiddenInput("client_id")?.value).toBe("client-abc");
    expect(hiddenInput("redirect_uri")?.value).toBe(
      "https://app.example.com/callback",
    );
    expect(hiddenInput("scope")?.value).toBe(VALID.scope);
    expect(hiddenInput("state")?.value).toBe("state-123");
    expect(hiddenInput("code_challenge")?.value).toBe("challenge-xyz");
    expect(hiddenInput("code_challenge_method")?.value).toBe("S256");
    expect(hiddenInput("consent_request")?.value).toBe(
      "signed-consent-request-token",
    );
    expect(hiddenInput("nonce")?.value).toBe("nonce-456");
    expect(hiddenInput("allow_all_services")?.value).toBe("false");
    expect(hiddenInput("allowed_service_ids")).toBeNull();
  });

  it("omits optional external-subject hidden inputs unless provided", () => {
    setSearch(VALID);

    render(<OAuthConsentPage />);

    // prompt + external_subject_* are conditionally rendered; absent here.
    expect(hiddenInput("prompt")).toBeNull();
    expect(hiddenInput("external_subject_platform")).toBeNull();
    expect(hiddenInput("external_subject_tenant")).toBeNull();
    expect(hiddenInput("external_subject_external_user_id")).toBeNull();
  });

  it("includes optional hidden inputs when their params are present", () => {
    setSearch({
      ...VALID,
      prompt: "consent",
      external_subject_platform: "telegram",
      external_subject_tenant: "tenant-1",
      external_subject_external_user_id: "ext-user-9",
    });

    render(<OAuthConsentPage />);

    expect(hiddenInput("prompt")?.value).toBe("consent");
    expect(hiddenInput("external_subject_platform")?.value).toBe("telegram");
    expect(hiddenInput("external_subject_tenant")?.value).toBe("tenant-1");
    expect(hiddenInput("external_subject_external_user_id")?.value).toBe(
      "ext-user-9",
    );
  });

  it("submits only the selected requested resources", () => {
    const resourceA = "https://nyx.example/api/v1/proxy/s/openai";
    const resourceB = "https://nyx.example/api/v1/proxy/s/anthropic";
    setSearch({ ...VALID, resource: [resourceA, resourceB] });

    render(<OAuthConsentPage />);

    expect(hiddenInput("resource_selection_present")?.value).toBe("true");
    expect(hiddenInputs("resource").map((input) => input.value)).toEqual([
      resourceA,
      resourceB,
    ]);

    fireEvent.click(screen.getByRole("checkbox", { name: resourceB }));

    expect(hiddenInputs("resource").map((input) => input.value)).toEqual([
      resourceA,
    ]);
  });

  it("renders an Unknown redirect host for an unparseable redirect_uri", () => {
    setSearch({ ...VALID, redirect_uri: "not a url" });

    render(<OAuthConsentPage />);

    // parseHost() catches the URL error and returns "Unknown" as the host,
    // rendered in the "Redirect host:" line of the verification block.
    expect(screen.getByText("Unknown")).toBeInTheDocument();
  });

  it("defaults service access to no selected services", () => {
    setSearch(VALID);

    render(<OAuthConsentPage />);

    const allServices = screen.getByRole("switch", { name: "All services" });
    expect(allServices).toHaveAttribute("aria-checked", "false");
    expect(screen.getByText("openai")).toBeInTheDocument();
    expect(hiddenInput("allow_all_services")?.value).toBe("false");
    expect(hiddenInput("allowed_service_ids")).toBeNull();
  });

  it("hides service choices when all-services is enabled", async () => {
    const user = userEvent.setup();
    setSearch(VALID);

    render(<OAuthConsentPage />);

    await user.click(screen.getByRole("switch", { name: "All services" }));

    expect(screen.queryByText("openai")).not.toBeInTheDocument();
    expect(screen.queryByText("inactive")).not.toBeInTheDocument();
    expect(screen.queryByText("org-service")).not.toBeInTheDocument();
    expect(hiddenInput("allow_all_services")?.value).toBe("true");
  });

  it("submits selected service ids only when scoped access is chosen", async () => {
    const user = userEvent.setup();
    setSearch(VALID);

    render(<OAuthConsentPage />);

    await user.click(screen.getByRole("checkbox", { name: /openai/i }));

    const selected = document.querySelectorAll<HTMLInputElement>(
      'input[type="hidden"][name="allowed_service_ids"]',
    );
    expect(Array.from(selected).map((input) => input.value)).toEqual([
      "svc-openai",
    ]);
  });

  it("preselects services for requested resource indicators", async () => {
    const resourceA = "https://nyx.example/api/v1/proxy/s/openai";
    const resourceB = "https://nyx.example/api/v1/proxy/s/unknown";
    setSearch({ ...VALID, resource: [resourceA, resourceB] });

    render(<OAuthConsentPage />);

    expect(hiddenInput("allow_all_services")?.value).toBe("false");
    await waitFor(() => {
      const selected = document.querySelectorAll<HTMLInputElement>(
        'input[type="hidden"][name="allowed_service_ids"]',
      );
      expect(Array.from(selected).map((input) => input.value)).toEqual([
        "svc-openai",
      ]);
    });
  });
});
