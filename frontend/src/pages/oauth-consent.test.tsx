import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import type { ReactElement } from "react";
import {
  cleanup,
  fireEvent,
  render as renderTesting,
  screen,
  waitFor,
  within,
} from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { vi } from "vitest";

import { OAuthConsentPage } from "./oauth-consent";
import { OAuthConsentPreviewPage } from "./oauth-consent-preview";
import { useAuthStore } from "@/stores/auth-store";

function render(page: ReactElement) {
  return renderTesting(
    <QueryClientProvider client={new QueryClient()}>{page}</QueryClientProvider>,
  );
}

const { state } = vi.hoisted(() => ({
  state: {
    userServices: [] as Array<{
      id: string;
      label: string;
      slug: string;
      catalog_service_name: string | null;
      catalog_service_description?: string | null;
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
    userServicesError: false,
  },
}));

vi.mock("@/hooks/use-user-services", () => ({
  useUserServices: () => ({
    data: state.userServices,
    isLoading: state.userServicesLoading,
    isError: state.userServicesError,
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
  useAuthStore.setState({ user: null });
  state.userServices = [
    {
      id: "svc-openai",
      label: "My OpenAI",
      slug: "openai-x2",
      catalog_service_name: "OpenAI",
      catalog_service_description:
        "Send model requests through your OpenAI connection.",
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
  state.userServicesError = false;
});

afterEach(() => {
  cleanup();
  window.history.pushState({}, "", "/");
  useAuthStore.setState({ user: null });
  vi.restoreAllMocks();
});

describe("OAuthConsentPage", () => {
  it("renders an inert preview using the consent interface", async () => {
    const user = userEvent.setup();
    render(<OAuthConsentPreviewPage />);

    expect(
      screen.getByText("Preview · Decisions disabled"),
    ).toBeInTheDocument();
    const serviceList = within(screen.getByRole("region", { name: "Service access list" }));
    expect(serviceList.getByText("Chrono Sandbox")).toBeInTheDocument();
    expect(serviceList.getByText("Chrono LLM (public)")).toBeInTheDocument();
    expect(screen.getByText(/Explore and test AI requests/)).toHaveClass(
      "line-clamp-2",
    );

    const allow = screen.getByRole("button", { name: "Allow access" });
    const decline = screen.getByRole("button", { name: "Decline" });
    expect(allow).toBeDisabled();
    expect(decline).toBeDisabled();
    expect(fireEvent.submit(allow.closest("form")!)).toBe(false);

    await user.click(screen.getByRole("button", { name: "Customize" }));
    expect(screen.getByLabelText("All services")).toBeInTheDocument();
    expect(screen.queryByText("Loading services...")).not.toBeInTheDocument();
  });

  it("fades only the clipped edges of the service list", async () => {
    const user = userEvent.setup();
    render(<OAuthConsentPreviewPage />);

    const summary = screen.getByRole("region", { name: "Service access list" });
    Object.defineProperties(summary, {
      clientHeight: { configurable: true, value: 200 },
      scrollHeight: { configurable: true, value: 600 },
      scrollTop: { configurable: true, writable: true, value: 0 },
    });
    const topFade = summary.parentElement!.querySelector(
      '[data-scroll-fade="top"]',
    );
    const bottomFade = summary.parentElement!.querySelector(
      '[data-scroll-fade="bottom"]',
    );

    fireEvent.scroll(summary);
    expect(topFade).toHaveClass("opacity-0");
    expect(bottomFade).toHaveClass("opacity-100");

    summary.scrollTop = 200;
    fireEvent.scroll(summary);
    expect(topFade).toHaveClass("opacity-100");
    expect(bottomFade).toHaveClass("opacity-100");

    summary.scrollTop = 400;
    fireEvent.scroll(summary);
    expect(topFade).toHaveClass("opacity-100");
    expect(bottomFade).toHaveClass("opacity-0");

    await user.click(screen.getByRole("button", { name: "Customize" }));
    expect(
      screen.getByRole("region", { name: "Service access list" }),
    ).toHaveClass("overflow-y-auto");
  });

  it("renders the invalid-request card when a required param is missing", () => {
    // Drop code_challenge -> `missing` is true.
    const { code_challenge, ...rest } = VALID;
    void code_challenge;
    setSearch(rest);

    render(<OAuthConsentPage />);

    expect(screen.getByText("Invalid consent request")).toBeInTheDocument();
    // The consent form must not render in the missing branch.
    expect(
      screen.queryByRole("button", { name: "Allow access" }),
    ).not.toBeInTheDocument();
  });

  it("shows readable permissions and keeps the exact scopes in app details", () => {
    setSearch(VALID);

    render(<OAuthConsentPage />);

    expect(screen.getByText("Authenticate your identity")).toBeInTheDocument();
    expect(screen.getByText("Long-lived access")).toBeInTheDocument();
    const permissions = screen.getByRole("region", {
      name: "Identity and app permissions",
    });
    const details = screen.getByText("App details").closest("details");
    expect(details).not.toBeNull();
    for (const scope of VALID.scope.split(" ")) {
      if (scope !== "offline_access") expect(within(permissions).getByText(scope)).toBeVisible();
      expect(details).toHaveTextContent(scope);
    }
  });

  it("identifies every scope requested by an MCP OAuth client", () => {
    setSearch({
      ...VALID,
      scope: "openid profile email roles groups offline_access proxy",
    });

    render(<OAuthConsentPage />);

    const permissions = screen.getByRole("region", {
      name: "Identity and app permissions",
    });
    expect(within(permissions).getByText("Your NyxID profile")).toBeVisible();
    expect(
      within(permissions).getByText("openid profile email roles groups"),
    ).toBeVisible();
    expect(within(screen.getByRole("region", { name: "Duration and revocation" })).getByText("offline_access")).toBeVisible();
    expect(
      within(permissions).queryByText("Custom permission"),
    ).not.toBeInTheDocument();
    // `proxy` is presented as the service access table.
    const services = screen.getByRole("region", { name: "Service access" });
    expect(within(services).getByText("proxy")).toBeVisible();
  });

  it("defaults agent apps to no service access and describes full access truthfully", () => {
    setSearch({
      ...VALID,
      scope: "openid profile email offline_access proxy",
    });

    const { container } = render(<OAuthConsentPage />);

    expect(
      screen.getByRole("radiogroup", { name: "All services" }),
    ).toBeVisible();
    expect(
      within(
        screen.getByRole("radiogroup", { name: "All services" }),
      ).getByRole("radio", { name: "Off" }),
    ).toHaveAttribute("aria-checked", "true");
    expect(
      container.querySelector('input[name="allow_all_services"]'),
    ).toHaveValue("false");
    expect(container.querySelector('input[name="write_access"]')).toBeNull();
    expect(
      screen.queryByRole("radio", { name: "Read" }),
    ).not.toBeInTheDocument();
    expect(
      screen.getByText(/including creating, updating or deleting data/),
    ).toBeVisible();
  });

  it("submits the supported all-services grant only after the user allows it", async () => {
    setSearch({
      ...VALID,
      scope: "openid profile email offline_access proxy",
    });

    const { container } = render(<OAuthConsentPage />);
    await userEvent.click(
      within(
        screen.getByRole("radiogroup", { name: "All services" }),
      ).getByRole("radio", { name: "Allow" }),
    );

    expect(
      container.querySelector('input[name="allow_all_services"]'),
    ).toHaveValue("true");
    expect(container.querySelector('input[name="write_access"]')).toBeNull();
    expect(
      container.querySelector('input[name="write_service_ids"]'),
    ).toBeNull();
  });

  it("submits only the allowed service and supports keyboard selection", async () => {
    setSearch({ ...VALID, scope: "openid proxy" });
    render(<OAuthConsentPage />);
    const group = screen.getByRole("radiogroup", { name: "My OpenAI" });
    within(group).getByRole("radio", { name: "Off" }).focus();
    await userEvent.keyboard("{ArrowRight}");
    expect(within(group).getByRole("radio", { name: "Allow" })).toHaveFocus();
    expect(hiddenInput("allow_all_services")).toHaveValue("false");
    expect(
      hiddenInputs("allowed_service_ids").map((input) => input.value),
    ).toEqual(["svc-openai"]);
    await userEvent.keyboard("{ArrowLeft}");
    expect(hiddenInputs("allowed_service_ids")).toHaveLength(0);
  });

  it("lets agent users turn off app-preselected services", async () => {
    setSearch({
      ...VALID,
      scope: "openid proxy",
      preselect_service_ids: ["svc-openai"],
    });
    render(<OAuthConsentPage />);
    expect(
      hiddenInputs("allowed_service_ids").map((input) => input.value),
    ).toEqual(["svc-openai"]);
    await userEvent.click(
      within(screen.getByRole("radiogroup", { name: "My OpenAI" })).getByRole(
        "radio",
        { name: "Off" },
      ),
    );
    expect(hiddenInputs("allowed_service_ids")).toHaveLength(0);
    expect(hiddenInput("allow_all_services")).toHaveValue("false");
  });

  it("shows the signed-in account in the connection card footer", () => {
    useAuthStore.setState({
      user: {
        id: "user-1",
        email: "alex@example.com",
        display_name: "Alex",
        avatar_url: null,
        email_verified: true,
        mfa_enabled: false,
        is_admin: false,
        is_active: true,
        created_at: "2026-01-01T00:00:00Z",
      },
    });
    setSearch({ ...VALID, client_name: "Atlas" });

    render(<OAuthConsentPage />);

    expect(screen.getByRole("img", { name: "NyxID" })).toBeInTheDocument();
    expect(
      screen.getByRole("img", { name: "NyxID" }).parentElement,
    ).toHaveClass("rounded-full");
    expect(
      screen.getByRole("img", { name: "NyxID" }).parentElement?.parentElement,
    ).toHaveTextContent("NyxID");
    expect(
      screen.queryByRole("img", { name: "NyxID connects to Atlas" }),
    ).not.toBeInTheDocument();
    expect(
      screen.getByText("alex@example.com").parentElement,
    ).toHaveTextContent("Signed in as alex@example.com");
    const heading = screen.getByRole("heading", {
      name: "Authorize application",
    });
    const subtitle = heading.nextElementSibling;
    expect(subtitle?.querySelector("span")).toHaveClass("text-foreground");
    expect(subtitle).toHaveTextContent(
      "Atlas wants to access your account via OAuth.",
    );
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

  it("shows catalog descriptions in the summary and service picker", async () => {
    const user = userEvent.setup();
    setSearch({ ...VALID, preselect_service_ids: ["svc-openai"] });

    render(<OAuthConsentPage />);

    const description = "Send model requests through your OpenAI connection.";
    expect(screen.getByText(description)).toHaveClass("line-clamp-2");

    await user.click(screen.getByRole("button", { name: "Customize" }));

    expect(screen.getByText(description)).toHaveClass("line-clamp-2");
    expect(
      screen.queryByText("No description provided"),
    ).not.toBeInTheDocument();
  });

  it("falls back to client_id as the display name when client_name is absent", () => {
    setSearch(VALID);

    render(<OAuthConsentPage />);

    // clientName = client_name || clientId => "client-abc" appears as the app name.
    expect(screen.getAllByText("client-abc").length).toBeGreaterThan(0);
  });

  it("shows unknown scopes directly without a generic permission label", () => {
    setSearch(VALID);

    render(<OAuthConsentPage />);

    expect(screen.getByText("Long-lived access")).toBeInTheDocument();
    const permissions = screen.getByRole("region", {
      name: "Identity and app permissions",
    });
    expect(within(permissions).getByText("custom:thing")).toHaveClass(
      "font-mono",
    );
    expect(
      within(permissions).queryByText("Custom permission"),
    ).not.toBeInTheDocument();
    expect(
      screen.getByText(/refresh tokens without asking/i),
    ).toBeInTheDocument();
    for (const rating of ["Low", "Medium", "High"]) {
      expect(screen.queryByText(rating)).not.toBeInTheDocument();
    }
  });

  it("describes broker binding as a durable credential", () => {
    setSearch({
      ...VALID,
      scope: "openid urn:nyxid:scope:broker_binding",
    });

    render(<OAuthConsentPage />);

    expect(screen.getByText("Durable broker access")).toBeInTheDocument();
    expect(
      screen.getByText(/durable NyxID credential that can act as you/i),
    ).toBeInTheDocument();
  });

  it("posts the consent decision form to /oauth/authorize/decision", () => {
    setSearch(VALID);

    render(<OAuthConsentPage />);

    const allow = screen.getByRole("button", { name: "Allow access" });
    const form = allow.closest("form")!;
    expect(form.getAttribute("action")).toBe("/oauth/authorize/decision");
    expect(form.getAttribute("method")?.toLowerCase()).toBe("post");
  });

  it("Allow access and Decline are submit buttons carrying the decision value", () => {
    setSearch(VALID);

    render(<OAuthConsentPage />);

    const allow = screen.getByRole("button", { name: "Allow access" });
    const deny = screen.getByRole("button", { name: "Decline" });

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
    expect(hiddenInput("browser_form")?.value).toBe("true");
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
    expect(within(screen.getByRole("region", { name: "Service access list" })).getByText("Org Service")).toBeInTheDocument();
    expect(within(screen.getByRole("region", { name: "Service access list" })).getByText("Organization · Acme Research")).toBeInTheDocument();
    expect(screen.queryByText("Viewer Org Service")).not.toBeInTheDocument();

    await user.click(screen.getByRole("switch", { name: "All services" }));

    expect(screen.queryByRole("checkbox", { name: /My OpenAI/ })).not.toBeInTheDocument();
    expect(screen.queryByText("Inactive")).not.toBeInTheDocument();
    expect(screen.queryByRole("checkbox", { name: /Org Service/ })).not.toBeInTheDocument();
    expect(hiddenInput("allow_all_services")?.value).toBe("true");
  });

  it("submits selected service ids only when scoped access is chosen", async () => {
    const user = userEvent.setup();
    setSearch(VALID);

    render(<OAuthConsentPage />);

    await user.click(screen.getByRole("button", { name: "Customize" }));
    await user.click(screen.getByRole("checkbox", { name: /My OpenAI/i }));
    expect(screen.getByRole("button", { name: "Done" })).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Save selection" }));
    expect(
      screen.getByRole("button", { name: "Customize" }),
    ).toBeInTheDocument();

    const selected = document.querySelectorAll<HTMLInputElement>(
      'input[type="hidden"][name="allowed_service_ids"]',
    );
    expect(Array.from(selected).map((input) => input.value)).toEqual([
      "svc-openai",
    ]);
  });

  it("also closes customization from Done without losing selections", async () => {
    const user = userEvent.setup();
    setSearch(VALID);
    render(<OAuthConsentPage />);

    await user.click(screen.getByRole("button", { name: "Customize" }));
    await user.click(screen.getByRole("checkbox", { name: /Org Service/i }));
    await user.click(screen.getByRole("button", { name: "Done" }));

    expect(
      screen.getByRole("button", { name: "Customize" }),
    ).toBeInTheDocument();
    expect(
      hiddenInputs("allowed_service_ids").map((input) => input.value),
    ).toEqual(["svc-org"]);
  });

  it("renders proxyable org services with org provenance and submits their ids", async () => {
    const user = userEvent.setup();
    setSearch(VALID);

    render(<OAuthConsentPage />);

    await user.click(screen.getByRole("button", { name: "Customize" }));
    await user.click(screen.getByRole("checkbox", { name: /Org Service/i }));

    expect(within(screen.getByRole("region", { name: "Service access list" })).getByText("Organization · Acme Research")).toBeInTheDocument();
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

    expect(within(screen.getByRole("region", { name: "Service access list" })).getByText("Org Service")).toBeInTheDocument();
    expect(within(screen.getByRole("region", { name: "Service access list" })).getByText("Organization · Acme Research")).toBeInTheDocument();
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

  it("keeps app default services selected on plain approve", () => {
    setSearch({ ...VALID, preselect_service_ids: ["svc-openai"] });

    render(<OAuthConsentPage />);

    // Summary lists the resolved default with the app-required badge; the
    // hidden inputs already carry the grant without any user interaction.
    expect(within(screen.getByRole("region", { name: "Service access list" })).getByText("My OpenAI")).toBeInTheDocument();
    expect(within(screen.getByRole("region", { name: "Service access list" })).getByText("Preselected by app")).toBeInTheDocument();
    expect(hiddenInput("allow_all_services")?.value).toBe("false");
    expect(hiddenInputs("allowed_service_ids").map((i) => i.value)).toEqual([
      "svc-openai",
    ]);
  });

  it("does not let the user remove an app default via Customize", async () => {
    const user = userEvent.setup();
    setSearch({ ...VALID, preselect_service_ids: ["svc-openai"] });

    render(<OAuthConsentPage />);

    await user.click(screen.getByRole("button", { name: "Customize" }));
    const appDefault = screen.getByRole("checkbox", { name: /My OpenAI/i });
    expect(appDefault).toBeChecked();
    expect(appDefault).toBeDisabled();
    expect(within(screen.getByRole("region", { name: "Service access list" })).getByText("Preselected by app")).toBeInTheDocument();

    await user.click(appDefault);
    expect(appDefault).toBeChecked();
    await user.click(screen.getByRole("checkbox", { name: /Org Service/i }));
    expect(hiddenInputs("allowed_service_ids").map((i) => i.value)).toEqual([
      "svc-openai",
      "svc-org",
    ]);
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
    const user = userEvent.setup();
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
    await user.click(screen.getByRole("button", { name: "Customize" }));
    expect(screen.getByRole("checkbox", { name: /My OpenAI/i })).toBeDisabled();
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
    expect(within(screen.getByRole("region", { name: "Service access list" })).getByText("My OpenAI")).toBeInTheDocument();
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

const DISCLOSURE_FIXTURE = {
  access_token_ttl_secs: 900,
  broker_access_token_ttl_secs: 300,
  refresh_token_ttl_secs: 604800,
  services: [
    {
      service_id: "svc-openai",
      credential_source: { type: "personal" },
      credential_kind: "oauth",
      upstream_scopes: [
        { scope: "repo", known: true, label: "Repository access", description: "Access to repositories.", sensitive: true },
        { scope: "provider:custom", known: false, label: null, description: null, sensitive: null },
      ],
      known_operations: { read: 2, write: 3, delete: 1, unclassified: 4, limiting: false },
      capabilities: { model_calls: true, binary_upload: null },
    },
    {
      service_id: "svc-org",
      credential_source: { type: "org", org_name: "Acme Research" },
      credential_kind: "api_key",
      upstream_scopes: null,
      known_operations: null,
      capabilities: { model_calls: null, binary_upload: null },
    },
  ],
};

function stubConsentReads(payload: Response, existing?: string[], existingOptions?: {
  allowAll?: boolean;
  expiry?: string;
}) {
  useAuthStore.setState({
    user: {
      id: "consent-reader",
      email: "person@example.com",
      display_name: null,
      avatar_url: null,
      email_verified: true,
      mfa_enabled: false,
      is_admin: false,
      is_active: true,
      created_at: "2026-01-01T00:00:00Z",
    },
  });
  return vi.spyOn(globalThis, "fetch").mockImplementation(async (input) => {
    if (String(input).endsWith("/consent-disclosure")) return payload;
    if (String(input).endsWith("/authorization") && existing) {
      return Response.json({
        allow_all_services: existingOptions?.allowAll ?? false,
        allowed_service_ids: existing,
        expires_at: existingOptions?.expiry ?? null,
      });
    }
    return Response.json({ error: "not_found" }, { status: 404 });
  });
}

describe("OAuth consent disclosure", () => {
  it("distinguishes a failed inventory read from a successful miss", () => {
    state.userServices = [];
    state.userServicesError = true;
    setSearch({ ...VALID, required_service_ids: ["unresolved-service-id"] });
    render(<OAuthConsentPage />);
    const summary = document.getElementById("oauth-submitted-service-access")!;
    expect(summary).toHaveTextContent("Service details could not be loaded.");
    expect(summary).not.toHaveTextContent("Deleted service");
    expect(summary).not.toHaveTextContent("unresolved-service-id");
  });

  it("shows retained all-service access separately from zero submitted services", async () => {
    stubConsentReads(Response.json(DISCLOSURE_FIXTURE), [], { allowAll: true });
    setSearch({ ...VALID, scope: "openid proxy" });
    render(<OAuthConsentPage />);
    const allow = screen.getByRole("button", { name: "Allow access" });
    const summary = within(document.getElementById(allow.getAttribute("aria-describedby")!)!);
    expect(await summary.findByText("Previously authorized access to all services stays authorized.")).toBeVisible();
    expect(summary.getByText("No services are included in this request.")).toBeVisible();
    const decision = new FormData(allow.closest("form")!);
    expect(decision.get("allow_all_services")).toBe("false");
    expect(decision.getAll("allowed_service_ids")).toEqual([]);
  });

  it("does not describe expired consent as retained", async () => {
    const reader = stubConsentReads(Response.json(DISCLOSURE_FIXTURE), ["svc-openai"], {
      allowAll: true,
      expiry: "2000-01-01T00:00:00Z",
    });
    setSearch({ ...VALID, scope: "openid proxy" });
    render(<OAuthConsentPage />);
    await screen.findByText(/configured lifetime for standard access tokens/);
    expect(reader.mock.calls.some(([url]) => String(url).endsWith("/authorization"))).toBe(true);
    const summary = document.getElementById("oauth-submitted-service-access")!;
    expect(summary).toHaveTextContent("No services are included in this request.");
    expect(summary).not.toHaveTextContent(/stay[s]? authorized/i);
  });

  it("keeps unresolved service IDs in the submission without exposing them in the summary", () => {
    const unavailableId = "7d902937-fc5a-4117-93c2-51d994999069";
    setSearch({ ...VALID, required_service_ids: [unavailableId] });
    render(<OAuthConsentPage />);
    const allow = screen.getByRole("button", { name: "Allow access" });
    expect(screen.getAllByText("Service details unavailable")).toHaveLength(2);
    expect(screen.getByRole("main")).not.toHaveTextContent(unavailableId);
    expect(new FormData(allow.closest("form")!).getAll("allowed_service_ids")).toEqual([unavailableId]);
  });

  it("uses one visible-service batch and synchronizes the summary with keyboard selection and the submitted form", async () => {
    const user = userEvent.setup();
    const request = stubConsentReads(Response.json(DISCLOSURE_FIXTURE), ["svc-org"]);
    setSearch({
      ...VALID,
      scope: "openid offline_access proxy",
      preselect_service_ids: ["svc-openai"],
    });
    render(<OAuthConsentPage />);
    const allow = screen.getByRole("button", { name: "Allow access" });
    const summary = document.getElementById(allow.getAttribute("aria-describedby")!)!;
    const picker = screen.getByRole("region", { name: "Service permissions" });
    expect(summary).toHaveAttribute("aria-live", "polite");
    expect(picker).not.toContainElement(summary);
    expect(summary.compareDocumentPosition(allow) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();

    expect(await within(summary).findByText("Repository access")).toBeVisible();
    expect(within(summary).getByText("Sensitive")).toBeVisible();
    expect(within(summary).getByText("provider:custom")).toBeVisible();
    expect(summary).toHaveTextContent("2 read, 3 write, 1 delete, and 4 unclassified");
    expect(summary).toHaveTextContent("These counts do not limit access.");
    expect(summary).toHaveTextContent("The app may send data to the model provider.");
    expect(summary).toHaveTextContent("Preselected by app");
    expect(summary).toHaveTextContent("Org Service");
    expect(summary).toHaveTextContent("stay authorized from an earlier approval");
    expect(within(picker).getByText(/provider controls what this API key can do/)).toBeVisible();
    expect(screen.getByText("The configured lifetime for standard access tokens is 15 minutes.")).toBeVisible();
    expect(screen.getByText(/configured lifetime for broker access tokens is 5 minutes/)).toBeVisible();
    expect(screen.getByText(/each refresh token lasts 7 days/)).toBeVisible();
    const batch = request.mock.calls.filter(([url]) => String(url).endsWith("/consent-disclosure"));
    expect(batch).toHaveLength(1);
    expect(batch[0]?.[1]).toMatchObject({
      method: "POST",
      body: JSON.stringify({ service_ids: ["svc-openai", "svc-org"] }),
    });
    const form = allow.closest("form")!;
    expect(new FormData(form).getAll("allowed_service_ids")).toEqual(["svc-openai"]);

    const org = within(screen.getByRole("radiogroup", { name: "Org Service" }));
    org.getByRole("radio", { name: "Off" }).focus();
    await user.keyboard("{ArrowRight}");
    expect(org.getByRole("radio", { name: "Allow" })).toHaveFocus();
    expect(within(summary).getByText("Already authorized")).toBeVisible();
    expect(new FormData(form).getAll("allowed_service_ids")).toEqual(["svc-openai", "svc-org"]);

    const all = within(screen.getByRole("radiogroup", { name: "All services" }));
    await user.click(all.getByRole("radio", { name: "Allow" }));
    expect(summary).toHaveTextContent("All current and future services are included.");
    expect(summary).toHaveTextContent("My OpenAI");
    expect(summary).toHaveTextContent("Access to repositories.");
    expect(summary).toHaveTextContent("Org Service");
    expect(new FormData(form).get("allow_all_services")).toBe("true");
    expect(new FormData(form).getAll("allowed_service_ids")).toEqual([]);

    const openai = within(screen.getByRole("radiogroup", { name: "My OpenAI" }));
    await user.click(openai.getByRole("radio", { name: "Off" }));
    expect(summary).not.toHaveTextContent("My OpenAI");
    expect(summary).not.toHaveTextContent("future services");
    const decision = new FormData(form, allow);
    expect(decision.get("allow_all_services")).toBe("false");
    expect(decision.getAll("allowed_service_ids")).toEqual(["svc-org"]);
    expect(decision.get("decision")).toBe("allow");
    expect(decision.get("consent_request")).toBe(VALID.consent_request);
    expect(decision.get("scope")).toBe("openid offline_access proxy");
    expect(decision.get("code_challenge")).toBe(VALID.code_challenge);
    expect(request.mock.calls.filter(([url]) => String(url).endsWith("/consent-disclosure"))).toHaveLength(1);
  });

  it.each(["404", "503", "legacy", "unknown"] as const)(
    "keeps full access and permits approval when disclosure is %s",
    async (kind) => {
      const payload = kind === "404" || kind === "503"
        ? Response.json({ error: "unavailable" }, { status: Number(kind) })
        : kind === "legacy"
          ? Response.json({ services: [{ id: "svc-openai" }] })
          : Response.json({
              ...DISCLOSURE_FIXTURE,
              services: [{ ...DISCLOSURE_FIXTURE.services[0], credential_kind: "future_credential" }],
            });
      const reader = stubConsentReads(payload);
      setSearch({ ...VALID, scope: "openid proxy", required_service_ids: ["svc-openai"] });
      render(<OAuthConsentPage />);
      await waitFor(() => expect(reader).toHaveBeenCalledWith(
        "/api/v1/user-services/consent-disclosure",
        expect.objectContaining({ method: "POST" }),
      ));
      const allow = screen.getByRole("button", { name: "Allow access" });
      const summary = document.getElementById(allow.getAttribute("aria-describedby")!)!;
      expect(allow).toBeEnabled();
      expect(summary).toHaveTextContent("Full access to the services listed below.");
      expect(summary).toHaveTextContent("Upstream permission details are unavailable.");
      expect(summary).toHaveTextContent("Required by app");
      expect(summary).not.toHaveTextContent("Repository access");
      expect(new FormData(allow.closest("form")!).getAll("allowed_service_ids")).toEqual(["svc-openai"]);
    },
  );
});
