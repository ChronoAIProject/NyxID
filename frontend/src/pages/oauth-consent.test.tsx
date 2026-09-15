import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { vi } from "vitest";

import { OAuthConsentPage } from "./oauth-consent";

const { state } = vi.hoisted(() => ({
  state: {
    userServices: [] as Array<{
      id: string;
      label: string;
      slug: string;
      catalog_service_name: string | null;
      resource_uri: string;
      auth_method: string;
      is_active: boolean;
      credential_source:
        | { type: "personal" }
        | {
            type: "org";
            org_id: string;
            org_name: string;
            role: "admin" | "member" | "viewer";
            allowed: boolean;
          };
    }>,
    userServicesLoading: false,
    presentation: undefined as
      | import("@/schemas/oauth-consent").ConsentPresentation
      | undefined,
    appConnect: undefined as
      | import("@/schemas/app-connect-links").AppConnectLink
      | undefined,
  },
}));

vi.mock("@/hooks/use-consent-presentation", () => ({
  useConsentPresentation: () => ({
    data: state.presentation,
    isPending: false,
  }),
}));

vi.mock("@/hooks/use-app-connect-links", () => ({
  useAppConnectConsent: () => ({ data: state.appConnect, isPending: false }),
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
  // The fixture models an issued, signed request separately from later URL edits.
  const resources = qs.getAll("resource");
  state.presentation = {
    client_id: String(params.client_id ?? ""),
    client_name: state.appConnect?.client_name ?? "Registered app",
    redirect_uri: String(params.redirect_uri ?? ""),
    scope: String(params.scope ?? ""),
    resources,
    mandatory_service_ids:
      state.appConnect?.items.flatMap((item) =>
        item.user_service_id ? [item.user_service_id] : [],
      ) ??
      state.userServices
        .filter((service) => resources.includes(service.resource_uri))
        .map((service) => service.id),
    selectable_service_ids: state.userServices
      .filter(
        (service) =>
          service.is_active &&
          (service.credential_source.type === "personal" ||
            service.credential_source.allowed),
      )
      .map((service) => service.id),
    app_connect_link_id: qs.get("app_connect_link_id"),
  };
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
  state.appConnect = undefined;
  state.presentation = undefined;
  window.history.pushState({}, "", "/");
  state.userServices = [
    {
      id: "svc-openai",
      label: "My OpenAI",
      slug: "openai-x2",
      catalog_service_name: "OpenAI",
      resource_uri: "https://nyx.example/api/v1/proxy/s/openai",
      auth_method: "bearer",
      is_active: true,
      credential_source: { type: "personal" },
    },
    {
      id: "svc-inactive",
      label: "Inactive",
      slug: "inactive",
      catalog_service_name: null,
      resource_uri: "https://nyx.example/api/v1/proxy/s/inactive",
      auth_method: "bearer",
      is_active: false,
      credential_source: { type: "personal" },
    },
    {
      id: "svc-org",
      label: "Org Service",
      slug: "org-service",
      catalog_service_name: null,
      resource_uri: "https://nyx.example/api/v1/proxy/s/org-service",
      auth_method: "bearer",
      is_active: true,
      credential_source: {
        type: "org",
        org_id: "org-1",
        org_name: "Acme Research",
        role: "member",
        allowed: true,
      },
    },
    {
      id: "svc-viewer-org",
      label: "Viewer Org Service",
      slug: "viewer-org-service",
      catalog_service_name: null,
      resource_uri: "https://nyx.example/api/v1/proxy/s/viewer-org-service",
      auth_method: "bearer",
      is_active: true,
      credential_source: {
        type: "org",
        org_id: "org-2",
        org_name: "Read Only Org",
        role: "viewer",
        allowed: false,
      },
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

    expect(screen.getAllByText("Registered app").length).toBeGreaterThan(0);
    expect(screen.queryByText("My Cool App")).not.toBeInTheDocument();
    // parseHost("https://app.example.com/callback") === "app.example.com".
    expect(screen.getByText("app.example.com")).toBeInTheDocument();
    // Full client_id and redirect_uri are shown in their detail blocks.
    expect(screen.getByText("client-abc")).toBeInTheDocument();
    expect(
      screen.getByText("https://app.example.com/callback"),
    ).toBeInTheDocument();
  });

  it("uses the registered presentation name when the URL omits client_name", () => {
    setSearch(VALID);

    render(<OAuthConsentPage />);

    expect(screen.getAllByText("Registered app").length).toBeGreaterThan(0);
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

  it("renders broker binding scope as a high-risk durable credential", () => {
    setSearch({
      ...VALID,
      scope: "openid urn:nyxid:scope:broker_binding",
    });

    render(<OAuthConsentPage />);

    expect(screen.getByText("Durable broker access")).toBeInTheDocument();
    expect(screen.getAllByText("High").length).toBeGreaterThan(0);
    expect(
      screen.getByText(/durable NyxID credential that can act as you/i),
    ).toBeInTheDocument();
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

  it("keeps every requested resource backend-authoritative", () => {
    const resourceA = "https://nyx.example/api/v1/proxy/s/openai";
    const resourceB = "https://nyx.example/api/v1/proxy/s/anthropic";
    setSearch({ ...VALID, resource: [resourceA, resourceB] });

    render(<OAuthConsentPage />);

    expect(hiddenInputs("resource").map((input) => input.value)).toEqual([
      resourceA,
      resourceB,
    ]);
    expect(screen.queryByRole("checkbox", { name: resourceA })).toBeNull();
    expect(screen.queryByRole("checkbox", { name: resourceB })).toBeNull();
  });

  it("renders an Unknown redirect host for an unparseable redirect_uri", () => {
    setSearch({ ...VALID, redirect_uri: "not a url" });

    render(<OAuthConsentPage />);

    // parseHost() catches the URL error and returns "Unknown" as the host,
    // rendered in the "Redirect host:" line of the verification block.
    expect(screen.getByText("Unknown")).toBeInTheDocument();
  });

  it("defaults to a sign-in-only summary with no picker rendered", () => {
    setSearch(VALID);

    render(<OAuthConsentPage />);

    expect(
      screen.getByText(/No service access requested/i),
    ).toBeInTheDocument();
    // Picker is collapsed by default: no checkboxes, no All-services switch.
    expect(
      screen.queryByRole("switch", { name: "All services" }),
    ).not.toBeInTheDocument();
    expect(screen.queryByText("My OpenAI")).not.toBeInTheDocument();
    expect(hiddenInput("allow_all_services")?.value).toBe("false");
    expect(hiddenInput("allowed_service_ids")).toBeNull();
  });

  it("reveals the picker behind Customize and hides choices when all-services is enabled", async () => {
    const user = userEvent.setup();
    setSearch(VALID);

    render(<OAuthConsentPage />);

    await user.click(screen.getByRole("button", { name: "Customize" }));
    expect(screen.getByText("My OpenAI")).toBeInTheDocument();
    expect(screen.getByText("Org Service")).toBeInTheDocument();
    expect(screen.getByText("Acme Research")).toBeInTheDocument();
    expect(screen.queryByText("Viewer Org Service")).not.toBeInTheDocument();

    await user.click(screen.getByRole("switch", { name: "All services" }));

    expect(screen.queryByText("My OpenAI")).not.toBeInTheDocument();
    expect(screen.queryByText("Inactive")).not.toBeInTheDocument();
    expect(screen.queryByText("Org Service")).not.toBeInTheDocument();
    expect(hiddenInput("allow_all_services")?.value).toBe("true");
  });

  it("submits selected service ids only when scoped access is chosen", async () => {
    const user = userEvent.setup();
    setSearch(VALID);

    render(<OAuthConsentPage />);

    await user.click(screen.getByRole("button", { name: "Customize" }));
    await user.click(screen.getByRole("checkbox", { name: /My OpenAI/i }));

    const selected = document.querySelectorAll<HTMLInputElement>(
      'input[type="hidden"][name="allowed_service_ids"]',
    );
    expect(Array.from(selected).map((input) => input.value)).toEqual([
      "svc-openai",
    ]);
  });

  it("renders proxyable org services with org provenance and submits their ids", async () => {
    const user = userEvent.setup();
    setSearch(VALID);

    render(<OAuthConsentPage />);

    await user.click(screen.getByRole("button", { name: "Customize" }));
    await user.click(screen.getByRole("checkbox", { name: /Org Service/i }));

    expect(screen.getByText("Acme Research")).toBeInTheDocument();
    expect(screen.getByText("Org")).toBeInTheDocument();
    expect(hiddenInputs("allowed_service_ids").map((i) => i.value)).toEqual([
      "svc-org",
    ]);
  });

  it("preselects requested org resource indicators and labels them in the summary", () => {
    setSearch({
      ...VALID,
      resource: ["https://nyx.example/api/v1/proxy/s/org-service"],
    });

    render(<OAuthConsentPage />);

    expect(screen.getByText("Org Service")).toBeInTheDocument();
    expect(screen.getByText("Acme Research")).toBeInTheDocument();
    expect(hiddenInputs("allowed_service_ids").map((i) => i.value)).toEqual([
      "svc-org",
    ]);
  });

  it("renders service display names, not raw slugs, in the picker", async () => {
    const user = userEvent.setup();
    setSearch(VALID);

    render(<OAuthConsentPage />);

    await user.click(screen.getByRole("button", { name: "Customize" }));

    // Primary text is the label; catalog name + slug appear as secondary.
    expect(screen.getByText("My OpenAI")).toBeInTheDocument();
    expect(screen.getByText("OpenAI · openai-x2")).toBeInTheDocument();
  });

  it("pre-selects app default services from preselect_service_ids and grants them on plain approve", () => {
    setSearch({ ...VALID, preselect_service_ids: ["svc-openai"] });

    render(<OAuthConsentPage />);

    // Summary lists the resolved default with the app-requested badge; the
    // hidden inputs already carry the grant without any user interaction.
    expect(screen.getByText("My OpenAI")).toBeInTheDocument();
    expect(screen.getByText("Requested by app")).toBeInTheDocument();
    expect(hiddenInput("allow_all_services")?.value).toBe("false");
    expect(hiddenInputs("allowed_service_ids").map((i) => i.value)).toEqual([
      "svc-openai",
    ]);
  });

  it("lets the user remove an app default via Customize", async () => {
    const user = userEvent.setup();
    setSearch({ ...VALID, preselect_service_ids: ["svc-openai"] });

    render(<OAuthConsentPage />);

    await user.click(screen.getByRole("button", { name: "Customize" }));
    await user.click(screen.getByRole("checkbox", { name: /My OpenAI/i }));

    expect(hiddenInput("allowed_service_ids")).toBeNull();
  });

  it("shows unmatched app defaults as informational rows", () => {
    setSearch({ ...VALID, unmatched_defaults: ["Lark Bot"] });

    render(<OAuthConsentPage />);

    expect(screen.getByText("Lark Bot")).toBeInTheDocument();
    expect(
      screen.getByText(/no matching service in your account/i),
    ).toBeInTheDocument();
    expect(hiddenInput("allowed_service_ids")).toBeNull();
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

  it("opens a Lark binding review with the current service grant visible", () => {
    setSearch({
      ...VALID,
      external_subject_platform: "lark",
      external_subject_external_user_id: "ou-user-1",
      binding_grant_id: "a".repeat(64),
      binding_review: "true",
      current_binding_service_ids: ["svc-openai"],
    });

    render(<OAuthConsentPage />);

    expect(screen.getByText("Review Lark bot access")).toBeInTheDocument();
    expect(screen.getByText("My OpenAI")).toBeInTheDocument();
    expect(screen.getByText("Authorized now")).toBeInTheDocument();
    expect(
      screen.getByRole("button", { name: "Update access" }),
    ).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Cancel" })).toBeInTheDocument();
    expect(hiddenInput("binding_grant_id")?.value).toBe("a".repeat(64));
    expect(
      hiddenInputs("allowed_service_ids").map((input) => input.value),
    ).toEqual(["svc-openai"]);
  });

  it("keeps app-required services selected and lets the user add optional access", async () => {
    const user = userEvent.setup();
    state.userServices.push(
      {
        id: "svc-ornn",
        label: "Ornn Skills",
        slug: "ornn-api",
        catalog_service_name: "Ornn",
        resource_uri: "https://nyx.example/api/v1/proxy/s/ornn-api",
        auth_method: "bearer",
        is_active: true,
        credential_source: { type: "personal" },
      },
      {
        id: "svc-optional",
        label: "Optional Service",
        slug: "optional-service",
        catalog_service_name: null,
        resource_uri: "https://nyx.example/api/v1/proxy/s/optional-service",
        auth_method: "bearer",
        is_active: true,
        credential_source: { type: "personal" },
      },
    );
    setSearch({
      ...VALID,
      external_subject_platform: "lark",
      external_subject_external_user_id: "ou-user-1",
      binding_grant_id: "b".repeat(64),
      binding_review: "true",
      current_binding_service_ids: ["svc-openai"],
      required_service_ids: ["svc-openai", "svc-org", "svc-ornn"],
    });
    state.presentation!.mandatory_service_ids = [
      "svc-openai",
      "svc-org",
      "svc-ornn",
    ];

    render(<OAuthConsentPage />);

    for (const name of [/My OpenAI/i, /Org Service/i, /Ornn Skills/i]) {
      const required = screen.getByRole("checkbox", { name });
      expect(required).toBeChecked();
      expect(required).toBeDisabled();
    }

    await user.click(
      screen.getByRole("checkbox", { name: /Optional Service/i }),
    );

    expect(screen.getByText("New")).toBeInTheDocument();
    expect(
      hiddenInputs("allowed_service_ids").map((input) => input.value),
    ).toEqual(["svc-openai", "svc-org", "svc-ornn", "svc-optional"]);
  });

  it("lets a binding review remove an optional current service", async () => {
    const user = userEvent.setup();
    setSearch({
      ...VALID,
      external_subject_platform: "lark",
      external_subject_external_user_id: "ou-user-1",
      binding_grant_id: "c".repeat(64),
      binding_review: "true",
      current_binding_service_ids: ["svc-org"],
      required_service_ids: ["svc-openai"],
    });

    render(<OAuthConsentPage />);

    await user.click(screen.getByRole("checkbox", { name: /Org Service/i }));

    expect(
      hiddenInputs("allowed_service_ids").map((input) => input.value),
    ).toEqual(["svc-openai"]);
  });
});

it("binds required rows to the session display and keeps them selected", async () => {
  state.appConnect = {
    id: "bound-session",
    oauth_client_id: VALID.client_id,
    client_name: "Verified source app",
    logo_url: "/api/v1/branding/assets/09d0abe0-6f31-4c48-a582-a1c2e3f06836",
    verified: true,
    handoff_blurb: null,
    destination: "app.example.com",
    requirements_version: 3,
    status: "ready_for_consent",
    expires_at: new Date(Date.now() + 900_000).toISOString(),
    callback_url: null,
    consent_url: "/oauth-consent",
    origin: "authorize",
    grant_update_required: false,
    items: [
      {
        requirement_id: "llm",
        label: "LLM",
        optional: false,
        state: "met",
        readiness: "met",
        user_service_id: "svc-openai",
        slug: "openai-x2",
        resource_uri: null,
        owner_id: "person",
        connect_link_id: null,
        reason_code: null,
        claim: "Lists models",
        validated_at: new Date(Date.now() - 20_000).toISOString(),
        valid_until: new Date(Date.now() + 280_000).toISOString(),
        granted_to_caller: false,
        catalog_slugs: [],
        required_scopes: [],
        choices: [],
      },
    ],
  };
  setSearch({
    ...VALID,
    client_name: "Forged name",
    app_connect_link_id: "bound-session",
  });
  const user = userEvent.setup();
  render(<OAuthConsentPage />);
  expect(
    screen.getByText(/Required by Verified source app - verified 20s ago/),
  ).toBeInTheDocument();
  expect(hiddenInputs("allowed_service_ids").map((i) => i.value)).toContain(
    "svc-openai",
  );
  expect(hiddenInput("consent_request")?.value).toBe(VALID.consent_request);
  expect(hiddenInput("app_connect_result_id")).toBeNull();
  expect(
    screen.getByRole("img", { name: "Verified source app logo" }),
  ).toBeInTheDocument();
  expect(screen.getByText("Verified")).toBeInTheDocument();
  expect(
    screen.getByText("Secured by NyxID · app.example.com"),
  ).toBeInTheDocument();
  await user.click(screen.getByRole("button", { name: "Customize" }));
  expect(screen.getByRole("checkbox", { name: /My OpenAI/ })).toBeDisabled();
});

it("refuses to show an Allow action for a cancelled bound session", () => {
  setSearch({ ...VALID, app_connect_link_id: "cancelled-session" });
  render(<OAuthConsentPage />);
  expect(screen.getByText(/no longer ready for consent/)).toBeInTheDocument();
  expect(
    screen.queryByRole("button", { name: "Allow" }),
  ).not.toBeInTheDocument();
});

it("never makes a forged required-service hint mandatory or renders a forged app name", async () => {
  setSearch(VALID);
  const forged = new URLSearchParams(window.location.search);
  forged.set("client_name", "Trusted bank impersonation");
  forged.append("required_service_ids", "svc-org");
  window.history.replaceState({}, "", `/oauth/consent?${forged}`);
  const user = userEvent.setup();
  render(<OAuthConsentPage />);
  expect(
    screen.queryByText("Trusted bank impersonation"),
  ).not.toBeInTheDocument();
  expect(screen.getAllByText("Registered app").length).toBeGreaterThan(0);
  expect(hiddenInputs("allowed_service_ids").map((i) => i.value)).toContain(
    "svc-org",
  );
  expect(state.presentation!.mandatory_service_ids).toEqual([]);
  expect(screen.queryByText("Required by app")).not.toBeInTheDocument();
  await user.click(screen.getByRole("button", { name: "Customize" }));
  const choice = screen.getByRole("checkbox", { name: /Org Service/ });
  expect(choice).toBeEnabled();
  await user.click(choice);
  expect(hiddenInputs("allowed_service_ids").map((i) => i.value)).not.toContain(
    "svc-org",
  );
});
