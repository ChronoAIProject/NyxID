import { cleanup, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it, vi } from "vitest";
import { OAuthConsentPage } from "./oauth-consent";
vi.mock("@/hooks/use-user-services", () => ({ useUserServices: () => ({ data: [], isLoading: false, isError: false }) }));
vi.mock("@/components/shared/machine-grant-consent", () => ({ MachineGrantConsent: ({ onPrepared, onSelectionRequired }: { onPrepared: (token: string | null) => void; onSelectionRequired: (required: boolean) => void }) => <section><h2>Machine access fixture</h2><button onClick={() => { onSelectionRequired(true); onPrepared("prepared-one-time-token"); }}>Prepare fixture</button><button onClick={() => { onSelectionRequired(true); onPrepared(null); }}>Change fixture</button><button onClick={() => { onSelectionRequired(false); onPrepared(null); }}>Clear fixture</button></section> }));
function open(incremental: boolean, misleadingScope = false) {
  const request = {
    token_type: "oauth_consent_request", exp: Math.floor(Date.now() / 1000) + 600,
    response_type: "code", client_id: "client", redirect_uri: "https://client.example/callback",
    scope: "openid machine", code_challenge: "challenge", code_challenge_method: "S256", resource: [],
    ...(incremental ? { service_access_mode: "incremental", incremental_consent: { client_name: "Fixture app", current_scopes: "openid", scopes: "openid machine", current_service_ids: [], allow_all_services: false, required_service_ids: [] } } : {}),
  };
  const token = `header.${btoa(JSON.stringify(request))}.signature`;
  const search = new URLSearchParams({ response_type: "code", client_id: "client", client_name: "Fixture app", redirect_uri: request.redirect_uri, scope: misleadingScope ? "openid" : request.scope, code_challenge: "challenge", code_challenge_method: "S256", consent_request: token });
  window.history.replaceState({}, "", `/oauth-consent?${search}`);
  return render(<OAuthConsentPage />);
}
afterEach(cleanup);
describe("Machine approval in the existing OAuth pages", () => {
  it.each([false, true])("allows no selection but requires saving a nonempty selection (incremental=%s)", async (incremental) => {
    open(incremental);
    const allow = screen.getByRole("button", { name: incremental ? "Continue" : "Allow access" });
    expect(allow).toBeEnabled();
    expect(screen.queryByText("Use selected Machines")).not.toBeInTheDocument();
    const original = new FormData(allow.closest("form")!).get("consent_request");
    await userEvent.click(screen.getByRole("button", { name: "Change fixture" }));
    expect(allow).toBeDisabled();
    await userEvent.click(screen.getByRole("button", { name: "Prepare fixture" }));
    expect(allow).toBeEnabled();
    expect(new FormData(allow.closest("form")!).get("consent_request")).toBe("prepared-one-time-token");
    await userEvent.click(screen.getByRole("button", { name: "Change fixture" }));
    expect(allow).toBeDisabled();
    await userEvent.click(screen.getByRole("button", { name: "Clear fixture" }));
    expect(allow).toBeEnabled();
    expect(new FormData(allow.closest("form")!).get("consent_request")).toBe(original);
  });
  it("cannot hide Machine review by removing Machine from standard URL display hints", () => {
    open(false, true);
    expect(screen.getByRole("heading", { name: "Machine access fixture" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Allow access" })).toBeEnabled();
  });
});
