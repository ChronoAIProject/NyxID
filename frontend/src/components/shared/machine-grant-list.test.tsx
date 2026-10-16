import { cleanup, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import type { ReactNode } from "react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { ApiError } from "@/lib/api-client";
import { MachineGrantList } from "./machine-grant-list";
const state = vi.hoisted(() => ({ data: undefined as unknown, error: undefined as unknown, refetch: vi.fn(), fetchNextPage: vi.fn(), hasNextPage: false, isFetchingNextPage: false, liveData: undefined as unknown, liveError: undefined as unknown, liveFetching: false, liveRefetch: vi.fn(), liveRead: vi.fn() }));
vi.mock("@/hooks/use-machine-grants", async (importOriginal) => ({ ...(await importOriginal<typeof import("@/hooks/use-machine-grants")>()), useMachineGrantList: () => state, useMachineGrant: (source: unknown, enabled: boolean) => { state.liveRead(source, enabled); return { data: state.liveData, error: state.liveError, isFetching: state.liveFetching, refetch: state.liveRefetch }; } }));
vi.mock("@tanstack/react-router", () => ({ Link: ({ children, to, params }: { children: ReactNode; to: string; params?: { keyId?: string } }) => <a href={to.replace("$keyId", params?.keyId ?? "")}>{children}</a> }));
const assignment = { capabilities: { shell: true, files: true, browser: false, computer: false, developer_browser: false }, mode: "shared_legacy", revision: 2, saved_login_ids: [], file_roots: ["/workspace/project"], service_ids: ["service-one"] };
const grant = { id: "grant", source: { kind: "api_key", api_key_id: "parent-key", api_key_state_version_at_approval: 1 }, revision: 4, policy_revision: 3, status: "active", expires_at: "2099-01-01T00:00:00Z", created_at: "2026-01-01T00:00:00Z", updated_at: "2026-01-01T00:00:00Z", revoked_at: null, revoked_reason: null, assignments: { machine: assignment } };
beforeEach(() => { vi.clearAllMocks(); state.data = undefined; state.error = undefined; state.fetchNextPage = vi.fn(); state.hasNextPage = false; state.isFetchingNextPage = false; state.liveData = undefined; state.liveError = undefined; state.liveFetching = false; });
afterEach(cleanup);
describe("per-Machine external grant view", () => {
  it("renders actual parent identity and restrictions with the existing Key route", () => {
    state.data = { external_grants: [grant] };
    render(<MachineGrantList machineId="machine" />);
    expect(screen.getByText("parent-key")).toBeInTheDocument();
    expect(screen.getByText("/workspace/project")).toBeInTheDocument();
    expect(screen.getByText("service-one")).toBeInTheDocument();
    expect(screen.getByText("Revision 4 · policy 3")).toBeInTheDocument();
    expect(screen.getByRole("link", { name: "Manage parent Key and login credentials" })).toHaveAttribute("href", "/keys/api-key/parent-key");
    expect(screen.getByText(/an active grant alone does not establish either/)).toBeInTheDocument();
  });
  it("links OAuth authority to granted apps and labels expired grants accurately", () => {
    state.data = { external_grants: [{ ...grant, expires_at: "2000-01-01T00:00:00Z", source: { kind: "oauth", oauth_client_id: "oauth-client", consent_id: "consent", consent_revision: "revision" } }] };
    render(<MachineGrantList machineId="machine" />);
    expect(screen.getByText("Expired")).toBeInTheDocument();
    expect(screen.getByText("oauth-client")).toBeInTheDocument();
    expect(screen.getByRole("link", { name: "Manage OAuth consent and Machine permissions" })).toHaveAttribute("href", "/settings/consents");
  });
  it("does not turn an unmounted aggregate API into an empty authorization view", () => {
    state.error = new ApiError(404, { error: "not_found", error_code: 404, message: "Not found" });
    render(<MachineGrantList machineId="machine" />);
    expect(screen.getByText(/no empty authorization state has been inferred/)).toBeInTheDocument();
    expect(screen.queryByText(/No external grants were returned/)).not.toBeInTheDocument();
  });
  it("checks combined live validity only after an explicit source lookup", async () => {
    state.data = { external_grants: [grant] };
    state.liveData = { grant, source_and_grant_valid: true };
    render(<MachineGrantList machineId="machine" />);
    expect(screen.getByText("Source + grant: Not checked")).toBeInTheDocument();
    expect(screen.queryByText("Source + grant: Valid")).not.toBeInTheDocument();
    expect(state.liveRead).toHaveBeenLastCalledWith({ kind: "api_key", id: "parent-key" }, false);
    await userEvent.click(screen.getByRole("button", { name: "Check source and grant" }));
    expect(state.liveRead).toHaveBeenLastCalledWith({ kind: "api_key", id: "parent-key" }, true);
    expect(screen.getByText("Source + grant: Valid")).toBeInTheDocument();
    expect(screen.getByText(/does not report source validity separately/)).toBeInTheDocument();
  });
  it("renders independent source and grant validity with only allowlisted reasons", async () => {
    state.data = { external_grants: [grant] };
    state.liveData = {
      grant,
      source_valid: true,
      source_validity_reason: null,
      grant_valid: false,
      grant_validity_reason: "expired",
      source_and_grant_valid: false,
    };
    render(<MachineGrantList machineId="machine" />);
    await userEvent.click(screen.getByRole("button", { name: "Check source and grant" }));
    expect(screen.getByText("Source: Valid")).toBeInTheDocument();
    expect(screen.getByText("Grant validity: Expired")).toBeInTheDocument();
    expect(screen.queryByText("expired" , { selector: "p" })).not.toBeInTheDocument();
  });
  it("fetches a real older page when the aggregate supplies a cursor", async () => {
    state.data = { external_grants: [grant], next_cursor: "opaque" };
    state.hasNextPage = true;
    render(<MachineGrantList machineId="machine" />);
    await userEvent.click(screen.getByRole("button", { name: "Load older grants" }));
    expect(state.fetchNextPage).toHaveBeenCalledTimes(1);
  });
  it("labels legacy discovery as potentially incomplete when no pagination contract is supplied", () => {
    state.data = { external_grants: [grant] };
    render(<MachineGrantList machineId="machine" />);
    expect(screen.getByText(/This server has not provided grant pagination/)).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Load older grants" })).not.toBeInTheDocument();
  });
  it("does not retain independent grant validity after local expiry", async () => {
    const expiredGrant = { ...grant, expires_at: "2000-01-01T00:00:00Z" };
    state.data = { external_grants: [expiredGrant], next_cursor: null };
    state.liveData = { grant: expiredGrant, source_valid: true, source_validity_reason: null, grant_valid: true, grant_validity_reason: null, source_and_grant_valid: true };
    render(<MachineGrantList machineId="machine" />);
    await userEvent.click(screen.getByRole("button", { name: "Check source and grant" }));
    expect(screen.getByText("Source: Valid")).toBeInTheDocument();
    expect(screen.getByText("Grant validity: Expired")).toBeInTheDocument();
    expect(screen.queryByText("Grant validity: Valid")).not.toBeInTheDocument();
  });
  it.each([{ id: "replacement-grant" }, { revision: 5 }])("does not apply changed grant %j live validity to an older aggregate row", async (change) => {
    state.data = { external_grants: [grant] };
    state.liveData = { grant: { ...grant, ...change }, source_and_grant_valid: true };
    render(<MachineGrantList machineId="machine" />);
    await userEvent.click(screen.getByRole("button", { name: "Check source and grant" }));
    expect(screen.queryByText("Source + grant: Valid")).not.toBeInTheDocument();
    expect(screen.getByText(/changed or was replaced/)).toBeInTheDocument();
  });
  it("removes a checked validity label on refetch failure", async () => {
    state.data = { external_grants: [grant] };
    state.liveData = { grant, source_and_grant_valid: true };
    const view = render(<MachineGrantList machineId="machine" />);
    await userEvent.click(screen.getByRole("button", { name: "Check source and grant" }));
    expect(screen.getByText("Source + grant: Valid")).toBeInTheDocument();
    state.liveError = new ApiError(503, { error: "unavailable", error_code: 503, message: "Unavailable" });
    view.rerender(<MachineGrantList machineId="machine" />);
    expect(screen.queryByText("Source + grant: Valid")).not.toBeInTheDocument();
    expect(screen.getByText(/Could not verify this source and grant/)).toBeInTheDocument();
  });
});
