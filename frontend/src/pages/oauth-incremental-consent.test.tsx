import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { OAuthConsentPage } from "./oauth-consent";

const { serviceState } = vi.hoisted(() => ({
  serviceState: {
    data: [] as Array<Record<string, unknown>>,
    isLoading: false,
    isError: false,
  },
}));
vi.mock("@/hooks/use-user-services", () => ({
  useUserServices: () => serviceState,
}));

function request(overrides: Record<string, unknown> = {}) {
  return {
    exp: Math.floor(Date.now() / 1000) + 600,
    token_type: "oauth_consent_request",
    response_type: "code",
    client_id: "aevatar-client",
    redirect_uri: "https://aevatar.example/auth/callback",
    scope: "openid proxy offline_access",
    state: "return-to-channel",
    code_challenge: "challenge",
    code_challenge_method: "S256",
    service_access_mode: "incremental",
    resource: [],
    incremental_consent: {
      client_name: "Aevatar",
      current_scopes: "openid proxy offline_access",
      scopes: "openid proxy offline_access",
      current_service_ids: ["old-a", "old-b"],
      allow_all_services: false,
      required_service_ids: ["new-c", "new-d", "new-c"],
    },
    ...overrides,
  };
}

function open(payload = request(), hints: Record<string, string> = {}) {
  const bytes = new TextEncoder().encode(JSON.stringify(payload));
  const encoded = btoa(String.fromCharCode(...bytes))
    .replace(/=/g, "")
    .replace(/\+/g, "-")
    .replace(/\//g, "_");
  const search = new URLSearchParams({
    consent_request: `header.${encoded}.signature`,
    ...hints,
  });
  window.history.replaceState({}, "", `/oauth-consent?${search}`);
  return render(<OAuthConsentPage />);
}

beforeEach(() => {
  serviceState.isLoading = false;
  serviceState.isError = false;
  serviceState.data = ["old-a", "old-b", "new-c", "new-d", "optional-e"].map(
    (id) => ({
      id,
      label: id,
      slug: `${id}-slug`,
      catalog_service_name: null,
      is_active: true,
      resource_uri: `https://nyx.example/api/v1/proxy/s/${id}-slug`,
      credential_source: { type: "personal" },
    }),
  );
});

describe("incremental consent", () => {
  it("counts only additions and keeps existing access immutable", () => {
    open();
    expect(
      screen.getByRole("heading", { name: "Update service access" }),
    ).toBeInTheDocument();
    expect(screen.getByText("Allow 2 additional services")).toBeInTheDocument();
    expect(screen.getByText("Already authorized")).toBeInTheDocument();
    expect(screen.getAllByText("Required")).toHaveLength(2);
    expect(screen.getAllByText("Authorized", { exact: true })).toHaveLength(2);
    expect(
      screen.getByRole("button", { name: "Allow 2 services" }),
    ).toBeEnabled();
    expect(
      screen.queryByRole("switch", { name: "All services" }),
    ).not.toBeInTheDocument();
    expect(
      screen.queryByRole("checkbox", { name: /old-a/ }),
    ).not.toBeInTheDocument();
    const form = screen
      .getByRole("button", { name: "Allow 2 services" })
      .closest("form")!;
    expect(new FormData(form).getAll("allowed_service_ids")).toEqual([
      "old-a",
      "old-b",
      "new-c",
      "new-d",
    ]);
    expect(form).toHaveAttribute(
      "action",
      "/oauth/authorize/incremental/decision",
    );
  });

  it("updates the count and submitted additions for optional selections", async () => {
    const user = userEvent.setup();
    open();
    await user.click(screen.getByText("Add optional services"));
    const optional = screen.getByRole("checkbox", { name: /optional-e/ });
    await user.click(optional);
    expect(
      screen.getByRole("button", { name: "Allow 3 services" }),
    ).toBeEnabled();
    await user.click(optional);
    expect(
      screen.getByRole("button", { name: "Allow 2 services" }),
    ).toBeEnabled();
  });

  it("uses signed display data and ignores forged URL hints including the mode", () => {
    open(request(), {
      client_name: "Fake trusted app",
      redirect_uri: "https://evil.example",
      service_access_mode: "replace",
      required_service_ids: "optional-e",
    });
    expect(screen.getAllByText("Aevatar").length).toBeGreaterThan(0);
    expect(screen.getByText("Return to aevatar.example")).toBeInTheDocument();
    expect(screen.queryByText("Fake trusted app")).not.toBeInTheDocument();
    expect(
      screen.getByRole("button", { name: "Allow 2 services" }),
    ).toBeEnabled();
  });

  it("distinguishes personal and organization services with identical names", () => {
    serviceState.data[2] = { ...serviceState.data[2], label: "GitHub" };
    serviceState.data[3] = {
      ...serviceState.data[3],
      label: "GitHub",
      credential_source: {
        type: "org",
        org_name: "Engineering",
        allowed: true,
      },
    };
    open();
    expect(screen.getAllByText("GitHub")).toHaveLength(2);
    expect(screen.getByText("Organization · Engineering")).toBeInTheDocument();
    expect(screen.getByText("new-c-slug")).toBeInTheDocument();
    expect(screen.getByText("new-d-slug")).toBeInTheDocument();
  });

  it.each(["missing", "disabled", "forbidden"])(
    "blocks approval when a required service is %s",
    (kind) => {
      if (kind === "missing")
        serviceState.data = serviceState.data.filter(
          (service) => service.id !== "new-c",
        );
      if (kind === "disabled") serviceState.data[2]!.is_active = false;
      if (kind === "forbidden")
        serviceState.data[2]!.credential_source = {
          type: "org",
          org_name: "Former team",
          allowed: false,
        };
      open();
      expect(
        screen.getByText(/unavailable, disconnected, or no longer shared/),
      ).toBeInTheDocument();
      expect(
        screen.getByRole("button", { name: "Allow 2 services" }),
      ).toBeDisabled();
      expect(screen.getByRole("button", { name: "Cancel" })).toBeEnabled();
    },
  );

  it.each(["loading", "failed"])(
    "blocks approval while the list is %s",
    (kind) => {
      serviceState.isLoading = kind === "loading";
      serviceState.isError = kind === "failed";
      open();
      expect(
        screen.getByRole("button", { name: "Allow 2 services" }),
      ).toBeDisabled();
    },
  );

  it("retains an unrestricted grant without introducing additions", () => {
    const payload = request();
    payload.incremental_consent.allow_all_services = true;
    open(payload);
    expect(
      screen.getByText("No additional services needed"),
    ).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Continue" })).toBeEnabled();
    expect(
      document.querySelector<HTMLInputElement>(
        'input[name="allow_all_services"]',
      )?.value,
    ).toBe("true");
    expect(screen.queryByRole("checkbox")).not.toBeInTheDocument();
  });

  it("does not recount already authorized requested services", () => {
    const payload = request();
    payload.incremental_consent.required_service_ids = [
      "old-a",
      "old-b",
      "old-a",
    ];
    open(payload);
    expect(
      screen.getByText("No additional services needed"),
    ).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Continue" })).toBeEnabled();
  });

  it("shows expiry and blocks submission", () => {
    open(request({ exp: Math.floor(Date.now() / 1000) - 1 }));
    expect(
      screen.getByText(/authorization request has expired/),
    ).toBeInTheDocument();
    expect(
      screen.getByRole("button", { name: "Allow 2 services" }),
    ).toBeDisabled();
    expect(
      screen.getByRole("button", { name: "Return to application" }),
    ).toBeEnabled();
  });

  it("shows newly requested OAuth scopes even when no services are added", () => {
    const payload = request();
    payload.incremental_consent.required_service_ids = ["old-a"];
    payload.incremental_consent.scopes =
      "openid proxy offline_access account:write";
    open(payload);
    expect(
      screen.getByText("Allow 1 additional permission"),
    ).toBeInTheDocument();
    expect(
      screen.getByRole("button", { name: "Allow 1 permission" }),
    ).toBeEnabled();
  });

  it.each(["loading", "failed"])(
    "shows scope-only permissions while service inventory is %s",
    (kind) => {
      serviceState.isLoading = kind === "loading";
      serviceState.isError = kind === "failed";
      serviceState.data = [];
      const payload = request();
      payload.incremental_consent.required_service_ids = [];
      payload.incremental_consent.scopes =
        "openid proxy offline_access account:write";
      open(payload);
      expect(
        screen.getByText("Allow 1 additional permission"),
      ).toBeInTheDocument();
      expect(
        screen.getByRole("button", { name: "Allow 1 permission" }),
      ).toBeEnabled();
      expect(
        screen.queryByText(/Services could not be loaded/),
      ).not.toBeInTheDocument();
      expect(
        screen.getAllByText(
          /Previously authorized service details unavailable/,
        ),
      ).toHaveLength(2);
    },
  );

  it("keeps an unavailable existing service without blocking new approvals", () => {
    serviceState.data = serviceState.data.filter(
      (service) => service.id !== "old-b",
    );
    open();
    expect(
      screen.getByText(/Previously authorized service unavailable/),
    ).toBeInTheDocument();
    expect(
      screen.getByRole("button", { name: "Allow 2 services" }),
    ).toBeEnabled();
  });

  it("rejects incomplete incremental payloads without falling back to replacement mode", () => {
    open(request({ incremental_consent: null }));
    expect(screen.getByText(/Invalid consent request/)).toBeInTheDocument();
    expect(
      screen.queryByRole("button", { name: /^Allow/ }),
    ).not.toBeInTheDocument();
  });

  it("submits cancel as a deny decision", () => {
    open();
    const cancel = screen.getByRole("button", { name: "Cancel" });
    expect(cancel).toHaveAttribute("name", "decision");
    expect(cancel).toHaveAttribute("value", "deny");
  });
});
