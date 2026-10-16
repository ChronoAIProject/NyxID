import { cleanup, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { ApiError } from "@/lib/api-client";
import { MachineGrantConsent } from "./machine-grant-consent";
const mocks = vi.hoisted(() => ({ prepare: vi.fn(), changed: vi.fn() }));
vi.mock("@/hooks/use-machine-grants", async (importOriginal) => ({ ...(await importOriginal<typeof import("@/hooks/use-machine-grants")>()), prepareMachineConsent: mocks.prepare }));
vi.mock("./machine-grant-editor", () => ({ MachineGrantEditor: ({ onSubmit, onSelectionChange }: { onSubmit: (value: object) => Promise<void>; onSelectionChange: () => void }) => <><button onClick={() => void onSubmit({ assignments: {} })}>Save fixture selection</button><button onClick={onSelectionChange}>Change fixture selection</button></> }));
function token(client = "client", scope = "openid machine", exp = Math.floor(Date.now() / 1000) + 900) { return `header.${btoa(JSON.stringify({ token_type: "oauth_consent_request", client_id: client, scope, exp }))}.signature`; }
beforeEach(() => vi.clearAllMocks());
afterEach(cleanup);
describe("protected Machine consent", () => {
  it("prepares the selection with the original protected token and clears approval when edited", async () => {
    const original = token(); const prepared = token("client", "openid proxy machine");
    mocks.prepare.mockResolvedValue(prepared);
    render(<MachineGrantConsent consentRequest={original} onPrepared={mocks.changed} />);
    await userEvent.click(screen.getByRole("button", { name: "Save fixture selection" }));
    await waitFor(() => expect(mocks.changed).toHaveBeenLastCalledWith(prepared));
    expect(mocks.prepare).toHaveBeenCalledWith(original, { assignments: {} });
    expect(screen.getByRole("status")).toHaveTextContent("Machine selection saved");
    await userEvent.click(screen.getByRole("button", { name: "Change fixture selection" }));
    expect(mocks.changed).toHaveBeenLastCalledWith(null);
    expect(screen.queryByRole("status")).not.toBeInTheDocument();
  });
  it("does not accept a response for another client", async () => {
    mocks.prepare.mockResolvedValue(token("other-client"));
    render(<MachineGrantConsent consentRequest={token()} onPrepared={mocks.changed} />);
    await userEvent.click(screen.getByRole("button", { name: "Save fixture selection" }));
    await screen.findByText(/could not be verified for this request/);
    expect(mocks.changed).not.toHaveBeenCalledWith(expect.any(String));
  });
  it("reports unmounted preparation accurately and keeps approval unavailable", async () => {
    mocks.prepare.mockRejectedValue(new ApiError(404, { error: "not_found", error_code: 404, message: "Not found" }));
    render(<MachineGrantConsent consentRequest={token()} onPrepared={mocks.changed} />);
    await userEvent.click(screen.getByRole("button", { name: "Save fixture selection" }));
    await screen.findByText(/Machine consent is not available on this server yet/);
    expect(mocks.changed).not.toHaveBeenCalledWith(expect.any(String));
  });
  it("does not offer preparation for an expired or non-Machine signed request", () => {
    render(<MachineGrantConsent consentRequest={token("client", "openid", 1)} onPrepared={mocks.changed} />);
    expect(screen.getByText(/protected Machine consent request is missing or expired/)).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Save fixture selection" })).not.toBeInTheDocument();
  });
});
