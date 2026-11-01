import { cleanup, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it, vi } from "vitest";
import { OAuthConsentPage } from "./oauth-consent";

const mocks = vi.hoisted(() => ({ nodes: [] as unknown[] }));
vi.mock("@/hooks/use-nodes", () => ({ useNodes: () => ({ data: mocks.nodes, isLoading: false, error: null }) }));
vi.mock("@/hooks/use-user-services", () => ({ useUserServices: () => ({ data: [], isLoading: false, isError: false }) }));
vi.mock("@/stores/auth-store", () => ({ useAuthStore: (selector: (state: { user: { id: string; email: string } }) => unknown) => selector({ user: { id: "owner", email: "owner@example.com" } }) }));

afterEach(cleanup);
describe("optional Machines through the actual consent editor", () => {
  it.each([[false, false], [false, true], [true, false], [true, true]])("allows other scopes without sharing Machines (incremental=%s, inventory=%s)", async (incremental, inventory) => {
    mocks.nodes = inventory ? [{
      id: "f3447880-a63f-4569-841b-389a87a7e4e3", name: "Personal Machine",
      owner: { kind: "user", id: "owner" },
      machine: { shell: true, files: true, roots: ["/workspace"] },
    }] : [];
    const request = {
      token_type: "oauth_consent_request", exp: Math.floor(Date.now() / 1000) + 600,
      response_type: "code", client_id: "client", redirect_uri: "https://client.example/callback",
      scope: "openid proxy machine offline_access", code_challenge: "challenge", code_challenge_method: "S256", resource: [],
      ...(incremental ? { service_access_mode: "incremental", incremental_consent: { client_name: "Fixture app", current_scopes: "openid proxy offline_access", scopes: "openid proxy machine offline_access", current_service_ids: [], allow_all_services: false, required_service_ids: [] } } : {}),
    };
    const token = `header.${btoa(JSON.stringify(request))}.signature`;
    window.history.replaceState({}, "", `/oauth-consent?${new URLSearchParams({
      response_type: "code", client_id: "client", client_name: "Fixture app", redirect_uri: request.redirect_uri,
      scope: request.scope, code_challenge: "challenge", code_challenge_method: "S256", consent_request: token,
    })}`);
    render(<OAuthConsentPage />);
    const allow = screen.getByRole("button", { name: incremental ? "Continue" : "Allow access" });
    expect(allow).toBeEnabled();
    expect(screen.getByRole("heading", { name: "Machine access (optional)" })).toBeInTheDocument();
    expect(screen.getByText(/No Machines will be shared/)).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Save Machine selection" })).not.toBeInTheDocument();
    expect(screen.queryByRole("checkbox", { name: /I explicitly allow shared legacy/ })).not.toBeInTheDocument();
    expect(new FormData(allow.closest("form")!).get("consent_request")).toBe(token);
    if (inventory) {
      await userEvent.click(screen.getByRole("checkbox", { name: "Allow this Machine" }));
      expect(allow).toBeDisabled();
      expect(screen.getByRole("button", { name: "Save Machine selection" })).toBeInTheDocument();
      await userEvent.click(screen.getByRole("checkbox", { name: "Allow this Machine" }));
      expect(allow).toBeEnabled();
    } else {
      expect(screen.getByText(/No eligible personal Machines/)).toBeInTheDocument();
    }
  });
});
