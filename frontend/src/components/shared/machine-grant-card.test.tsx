import {
  cleanup,
  render,
  screen,
  waitFor,
  within,
} from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import type { ReactNode } from "react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { ApiError } from "@/lib/api-client";
import { MachineGrantCard } from "./machine-grant-card";
const mocks = vi.hoisted(() => ({
  data: undefined as unknown,
  error: undefined as unknown,
  fetching: false,
  save: vi.fn(),
  revoke: vi.fn(),
  refetch: vi.fn(),
}));
vi.mock("@tanstack/react-router", () => ({
  Link: ({ children }: { children: ReactNode }) => (
    <a href="/assistant/machines">{children}</a>
  ),
}));
vi.mock("@/hooks/use-api-keys", () => ({
  useApiKey: () => ({
    data: {
      state_version: 8,
      allow_all_nodes: false,
      allowed_node_ids: ["machine"],
      allow_all_services: false,
      allowed_service_ids: ["service-one"],
    },
    isPending: false,
    error: undefined,
  }),
}));
vi.mock("@/hooks/use-machine-grants", async (importOriginal) => ({
  ...(await importOriginal<typeof import("@/hooks/use-machine-grants")>()),
  useMachineGrant: () => ({
    data: mocks.data,
    error: mocks.error,
    isFetching: mocks.fetching,
    refetch: mocks.refetch,
  }),
  useSaveMachineGrant: () => ({ mutateAsync: mocks.save }),
  useRevokeMachineGrant: () => ({ mutateAsync: mocks.revoke }),
}));
vi.mock("./machine-grant-editor", () => ({
  MachineGrantEditor: ({
    onSubmit,
  }: {
    onSubmit: (value: object) => Promise<void>;
  }) => (
    <button onClick={() => void onSubmit({ assignments: {} })}>
      Submit fixture grant
    </button>
  ),
}));
function grant() {
  return {
    id: "grant",
    revision: 7,
    policy_revision: 3,
    status: "active",
    source: {
      kind: "api_key",
      api_key_id: "key",
      api_key_state_version_at_approval: 4,
    },
    expires_at: "2099-01-01T00:00:00Z",
    created_at: "2026-01-01T00:00:00Z",
    updated_at: "2026-01-01T00:00:00Z",
    revoked_at: null,
    revoked_reason: null,
    assignments: {},
  };
}
beforeEach(() => {
  vi.clearAllMocks();
  mocks.data = undefined;
  mocks.error = undefined;
  mocks.fetching = false;
  mocks.save.mockResolvedValue(grant());
  mocks.revoke.mockResolvedValue(grant());
});
afterEach(cleanup);
describe("Machine grant owner management", () => {
  it.each([
    ["missing", null, "missing"],
    ["revoked", { ...grant(), status: "revoked" }, "revoked"],
    ["expired", { ...grant(), expires_at: "2000-01-01T00:00:00Z" }, "expired"],
  ])(
    "allows approval for a valid Key source with a %s grant",
    async (_state, currentGrant, reason) => {
      mocks.data = {
        grant: currentGrant,
        source_valid: true,
        source_validity_reason: null,
        grant_valid: false,
        grant_validity_reason: reason,
        source_and_grant_valid: false,
      };
      render(
        <MachineGrantCard
          source={{ kind: "api_key", id: "key" }}
          stateVersion={8}
        />,
      );
      const approve = screen.getByRole("button", {
        name: "Approve Machine permissions",
      });
      expect(approve).toBeEnabled();
      expect(
        screen.queryByRole("button", { name: "Revoke Machine permissions" }),
      ).not.toBeInTheDocument();
      await userEvent.click(approve);
      expect(
        screen.getByRole("button", { name: "Submit fixture grant" }),
      ).toBeInTheDocument();
      await userEvent.click(
        screen.getByRole("button", { name: "Submit fixture grant" }),
      );
      await waitFor(() =>
        expect(mocks.save).toHaveBeenCalledWith({
          expected_grant_id: null,
          expected_revision: null,
          expected_state_version: 8,
          selection: { assignments: {} },
        }),
      );
    },
  );
  it("does not edit an active invalid grant even when its Key source is valid", () => {
    const assignment = {
      capabilities: {
        shell: true,
        files: true,
        browser: false,
        computer: false,
        developer_browser: false,
      },
      mode: "shared_legacy",
      revision: 1,
      saved_login_ids: [],
      file_roots: ["/workspace"],
      service_ids: [],
    };
    mocks.data = {
      grant: { ...grant(), assignments: { machine: assignment } },
      source_valid: true,
      source_validity_reason: null,
      grant_valid: false,
      grant_validity_reason: "changed",
      source_and_grant_valid: false,
    };
    render(
      <MachineGrantCard
        source={{ kind: "api_key", id: "key" }}
        stateVersion={8}
        machineId="machine"
      />,
    );
    expect(
      screen.getByRole("button", { name: "Edit current source grant" }),
    ).toBeDisabled();
    expect(
      screen.queryByRole("button", { name: "Remove current Machine" }),
    ).not.toBeInTheDocument();
    expect(
      screen.queryByRole("button", { name: "Submit fixture grant" }),
    ).not.toBeInTheDocument();
  });
  it("requires new approval rather than offering removal for an expired assignment", () => {
    const assignment = {
      capabilities: {
        shell: true,
        files: true,
        browser: false,
        computer: false,
        developer_browser: false,
      },
      mode: "shared_legacy",
      revision: 1,
      saved_login_ids: [],
      file_roots: ["/workspace"],
      service_ids: [],
    };
    mocks.data = {
      grant: {
        ...grant(),
        expires_at: "2000-01-01T00:00:00Z",
        assignments: { machine: assignment },
      },
      source_valid: true,
      source_validity_reason: null,
      grant_valid: false,
      grant_validity_reason: "expired",
      source_and_grant_valid: false,
    };
    render(
      <MachineGrantCard
        source={{ kind: "api_key", id: "key" }}
        machineId="machine"
      />,
    );
    expect(
      screen.getByRole("button", { name: "Approve Machine permissions" }),
    ).toBeEnabled();
    expect(
      screen.queryByRole("button", { name: "Remove current Machine" }),
    ).not.toBeInTheDocument();
    expect(
      screen.queryByRole("button", { name: "Revoke Machine permissions" }),
    ).not.toBeInTheDocument();
  });
  it.each([
    ["unknown", { source_and_grant_valid: false }],
    [
      "invalid",
      {
        source_valid: false,
        source_validity_reason: "disabled",
        grant_valid: false,
        grant_validity_reason: "missing",
        source_and_grant_valid: false,
      },
    ],
  ])(
    "does not approve a missing grant when source validity is %s",
    (_state, validity) => {
      mocks.data = { grant: null, ...validity };
      render(
        <MachineGrantCard
          source={{ kind: "api_key", id: "key" }}
          stateVersion={8}
        />,
      );
      expect(
        screen.getByRole("button", { name: "Approve Machine permissions" }),
      ).toBeDisabled();
      expect(
        screen.queryByRole("button", { name: "Submit fixture grant" }),
      ).not.toBeInTheDocument();
    },
  );
  it("treats 404 as unavailable rather than no grant", () => {
    mocks.error = new ApiError(404, {
      error: "not_found",
      error_code: 404,
      message: "Not found",
    });
    render(
      <MachineGrantCard
        source={{ kind: "api_key", id: "key" }}
        stateVersion={8}
      />,
    );
    expect(
      screen.getByText(/No permission state has been inferred/),
    ).toBeInTheDocument();
    expect(screen.queryByText("Grant: none")).not.toBeInTheDocument();
    expect(
      screen.queryByRole("button", { name: "Approve Machine permissions" }),
    ).not.toBeInTheDocument();
  });
  it("hides stale validity and management controls after a failed live refetch", () => {
    mocks.data = { grant: grant(), source_and_grant_valid: true };
    const view = render(
      <MachineGrantCard
        source={{ kind: "api_key", id: "key" }}
        stateVersion={8}
      />,
    );
    expect(screen.getByText("Source + grant: valid")).toBeInTheDocument();
    mocks.error = new ApiError(503, {
      error: "unavailable",
      error_code: 503,
      message: "Unavailable",
    });
    view.rerender(
      <MachineGrantCard
        source={{ kind: "api_key", id: "key" }}
        stateVersion={8}
      />,
    );
    expect(screen.queryByText("Source + grant: valid")).not.toBeInTheDocument();
    expect(screen.queryByText("Grant: active")).not.toBeInTheDocument();
    expect(
      screen.queryByRole("button", { name: "Edit Machine permissions" }),
    ).not.toBeInTheDocument();
    expect(
      screen.queryByRole("button", { name: "Revoke Machine permissions" }),
    ).not.toBeInTheDocument();
    expect(
      screen.getByText(/Could not verify Machine permissions/),
    ).toBeInTheDocument();
  });
  it("describes the live validity result as combined rather than independent source validity", () => {
    mocks.data = { grant: grant(), source_and_grant_valid: false };
    render(
      <MachineGrantCard
        source={{ kind: "api_key", id: "key" }}
        stateVersion={8}
      />,
    );
    expect(
      screen.getByText("Source + grant: not currently valid"),
    ).toBeInTheDocument();
    expect(screen.getByText("Grant: active")).toBeInTheDocument();
    expect(
      screen.getByText(/does not report source validity separately/),
    ).toBeInTheDocument();
    expect(screen.queryByText("Source: invalid")).not.toBeInTheDocument();
  });
  it("shows independent source and grant validity reasons when supplied", () => {
    mocks.data = {
      grant: grant(),
      source_valid: false,
      source_validity_reason: "disabled",
      grant_valid: true,
      grant_validity_reason: null,
      source_and_grant_valid: false,
    };
    render(
      <MachineGrantCard
        source={{ kind: "api_key", id: "key" }}
        stateVersion={8}
      />,
    );
    expect(screen.getByText("Source: Disabled")).toBeInTheDocument();
    expect(screen.getByText("Grant validity: Valid")).toBeInTheDocument();
    expect(
      screen.getByRole("button", { name: "Edit Machine permissions" }),
    ).toBeDisabled();
  });
  it("never keeps a positive grant validity after its authoritative expiry passes", () => {
    mocks.data = {
      grant: { ...grant(), expires_at: "2000-01-01T00:00:00Z" },
      source_valid: true,
      source_validity_reason: null,
      grant_valid: true,
      grant_validity_reason: null,
      source_and_grant_valid: true,
    };
    render(
      <MachineGrantCard
        source={{ kind: "api_key", id: "key" }}
        stateVersion={8}
      />,
    );
    expect(screen.getByText("Source: Valid")).toBeInTheDocument();
    expect(screen.getByText("Grant validity: Expired")).toBeInTheDocument();
    expect(screen.queryByText("Grant validity: Valid")).not.toBeInTheDocument();
    expect(
      screen.getByText("Source + grant: not currently valid"),
    ).toBeInTheDocument();
  });
  it("shows independent disabled source state even when the source adapter disallows approval", () => {
    mocks.data = {
      grant: grant(),
      source_valid: false,
      source_validity_reason: "disabled",
      grant_valid: true,
      grant_validity_reason: null,
      source_and_grant_valid: false,
    };
    render(
      <MachineGrantCard
        source={{ kind: "api_key", id: "key" }}
        eligible={false}
        stateVersion={8}
      />,
    );
    expect(screen.getByText("Source: Disabled")).toBeInTheDocument();
    expect(screen.getByText("Grant validity: Valid")).toBeInTheDocument();
    expect(
      screen.getByRole("button", { name: "Edit Machine permissions" }),
    ).toBeDisabled();
  });
  it("suppresses cached positive checks and editing while the source is being revalidated", () => {
    mocks.data = {
      grant: grant(),
      source_valid: true,
      source_validity_reason: null,
      grant_valid: true,
      grant_validity_reason: null,
      source_and_grant_valid: true,
    };
    mocks.fetching = true;
    render(
      <MachineGrantCard
        source={{ kind: "api_key", id: "key" }}
        stateVersion={8}
      />,
    );
    expect(screen.queryByText("Source: Valid")).not.toBeInTheDocument();
    expect(screen.queryByText("Grant validity: Valid")).not.toBeInTheDocument();
    expect(screen.getByText("Source + grant: checking")).toBeInTheDocument();
    expect(
      screen.getByRole("button", { name: "Edit Machine permissions" }),
    ).toBeDisabled();
  });
  it("blocks an already-open revoke dialog when live permission verification fails", async () => {
    mocks.data = { grant: grant(), source_and_grant_valid: true };
    const view = render(
      <MachineGrantCard
        source={{ kind: "api_key", id: "key" }}
        stateVersion={8}
      />,
    );
    await userEvent.click(
      screen.getByRole("button", { name: "Revoke Machine permissions" }),
    );
    mocks.error = new ApiError(503, {
      error: "unavailable",
      error_code: 503,
      message: "Unavailable",
    });
    view.rerender(
      <MachineGrantCard
        source={{ kind: "api_key", id: "key" }}
        stateVersion={8}
      />,
    );
    const confirm = screen.getByRole("button", {
      name: "Revoke Machine permissions",
    });
    expect(confirm).toBeDisabled();
    await userEvent.click(confirm);
    expect(mocks.revoke).not.toHaveBeenCalled();
  });
  it("requires a new revoke review after the grant revision changes", async () => {
    mocks.data = { grant: grant(), source_and_grant_valid: true };
    const view = render(
      <MachineGrantCard
        source={{ kind: "api_key", id: "key" }}
        stateVersion={8}
      />,
    );
    await userEvent.click(
      screen.getByRole("button", { name: "Revoke Machine permissions" }),
    );
    mocks.data = {
      grant: { ...grant(), revision: 8 },
      source_and_grant_valid: true,
    };
    view.rerender(
      <MachineGrantCard
        source={{ kind: "api_key", id: "key" }}
        stateVersion={8}
      />,
    );
    expect(
      screen.getByRole("button", { name: "Revoke Machine permissions" }),
    ).toBeDisabled();
    expect(screen.getByText(/changed since this review/)).toBeInTheDocument();
    expect(mocks.revoke).not.toHaveBeenCalled();
  });
  it("closes a prior source's editor when the source changes", async () => {
    mocks.data = { grant: grant(), source_and_grant_valid: true };
    const view = render(
      <MachineGrantCard
        source={{ kind: "api_key", id: "key" }}
        stateVersion={8}
      />,
    );
    await userEvent.click(
      screen.getByRole("button", { name: "Edit Machine permissions" }),
    );
    expect(
      screen.getByRole("button", { name: "Submit fixture grant" }),
    ).toBeInTheDocument();
    mocks.data = {
      grant: {
        ...grant(),
        id: "new-grant",
        source: {
          kind: "api_key",
          api_key_id: "other-key",
          api_key_state_version_at_approval: 1,
        },
      },
      source_and_grant_valid: true,
    };
    view.rerender(
      <MachineGrantCard
        source={{ kind: "api_key", id: "other-key" }}
        stateVersion={1}
      />,
    );
    expect(
      screen.queryByRole("button", { name: "Submit fixture grant" }),
    ).not.toBeInTheDocument();
  });
  it("sends grant identity, revision and current Key approval CAS together", async () => {
    mocks.data = { grant: grant(), source_and_grant_valid: true };
    render(
      <MachineGrantCard
        source={{ kind: "api_key", id: "key" }}
        stateVersion={8}
      />,
    );
    await userEvent.click(
      screen.getByRole("button", { name: "Edit Machine permissions" }),
    );
    await userEvent.click(
      screen.getByRole("button", { name: "Submit fixture grant" }),
    );
    await waitFor(() =>
      expect(mocks.save).toHaveBeenCalledWith({
        expected_grant_id: "grant",
        expected_revision: 7,
        expected_state_version: 8,
        selection: { assignments: {} },
      }),
    );
  });
  it("requires reload on a conflict without pretending the permissions were saved", async () => {
    mocks.data = { grant: grant(), source_and_grant_valid: true };
    mocks.save.mockRejectedValue(
      new ApiError(409, {
        error: "conflict",
        error_code: 409,
        message: "Changed",
      }),
    );
    render(
      <MachineGrantCard
        source={{ kind: "api_key", id: "key" }}
        stateVersion={8}
      />,
    );
    await userEvent.click(
      screen.getByRole("button", { name: "Edit Machine permissions" }),
    );
    await userEvent.click(
      screen.getByRole("button", { name: "Submit fixture grant" }),
    );
    await screen.findByText(/Reload and review the latest permissions/);
    expect(
      screen.queryByRole("button", { name: "Submit fixture grant" }),
    ).not.toBeInTheDocument();
    expect(
      screen.getByRole("button", { name: "Edit Machine permissions" }),
    ).toBeDisabled();
  });
  it("does not create OAuth grants from the existing consent management page", () => {
    mocks.data = {
      grant: null,
      source_valid: true,
      source_validity_reason: null,
      grant_valid: false,
      grant_validity_reason: "missing",
      source_and_grant_valid: false,
    };
    render(<MachineGrantCard source={{ kind: "oauth", id: "client" }} />);
    expect(
      screen.getByRole("button", { name: "Approve Machine permissions" }),
    ).toBeDisabled();
    expect(
      screen.getByText(/Reconnect from this application/),
    ).toBeInTheDocument();
  });
  it("does not offer editing without current parent Key CAS", () => {
    mocks.data = {
      grant: null,
      source_valid: true,
      source_validity_reason: null,
      grant_valid: false,
      grant_validity_reason: "missing",
      source_and_grant_valid: false,
    };
    render(<MachineGrantCard source={{ kind: "api_key", id: "key" }} />);
    expect(
      screen.getByRole("button", { name: "Approve Machine permissions" }),
    ).toBeDisabled();
    expect(
      screen.getByText(/parent Key revision required for safe approval/),
    ).toBeInTheDocument();
  });

  it("removes the current Machine with a complete remaining assignment map", async () => {
    const current = {
      capabilities: {
        shell: true,
        files: true,
        browser: false,
        computer: false,
        developer_browser: false,
      },
      mode: "shared_legacy",
      revision: 1,
      saved_login_ids: [],
      file_roots: ["/workspace"],
      service_ids: [],
    };
    const other = { ...current, revision: 2, file_roots: ["/other"] };
    mocks.data = {
      grant: { ...grant(), assignments: { machine: current, other: other } },
      source_and_grant_valid: true,
    };
    render(
      <MachineGrantCard
        source={{ kind: "api_key", id: "key" }}
        machineId="machine"
      />,
    );
    expect(
      screen.getByText("Key state version").nextElementSibling,
    ).toHaveTextContent("8");
    expect(
      screen.getByText("Node ceiling").nextElementSibling,
    ).toHaveTextContent("machine");
    await userEvent.click(
      screen.getByRole("button", { name: "Remove current Machine" }),
    );
    const dialog = screen.getByRole("dialog");
    expect(within(dialog).getByText(/other · retained/)).toBeInTheDocument();
    expect(
      within(dialog).getByRole("button", { name: "Remove current Machine" }),
    ).toBeDisabled();
    expect(mocks.save).not.toHaveBeenCalled();
    await userEvent.click(
      within(dialog).getByRole("checkbox", {
        name: /I explicitly allow shared legacy/,
      }),
    );
    await userEvent.click(
      within(dialog).getByRole("button", { name: "Remove current Machine" }),
    );
    await waitFor(() =>
      expect(mocks.save).toHaveBeenCalledWith(
        expect.objectContaining({
          expected_grant_id: "grant",
          expected_revision: 7,
          expected_state_version: 8,
          selection: expect.objectContaining({
            assignments: { other },
            separation: "shared_legacy",
            legacy_warning_acknowledged: true,
          }),
        }),
      ),
    );
    expect(mocks.revoke).not.toHaveBeenCalled();
  });

  it("revokes the whole source Machine grant when the current Machine is the last assignment", async () => {
    const current = {
      capabilities: {
        shell: true,
        files: true,
        browser: false,
        computer: false,
        developer_browser: false,
      },
      mode: "shared_legacy",
      revision: 1,
      saved_login_ids: [],
      file_roots: ["/workspace"],
      service_ids: [],
    };
    mocks.data = {
      grant: { ...grant(), assignments: { machine: current } },
      source_and_grant_valid: true,
    };
    render(
      <MachineGrantCard
        source={{ kind: "api_key", id: "key" }}
        machineId="machine"
      />,
    );
    await userEvent.click(
      screen.getByRole("button", { name: "Remove current Machine" }),
    );
    const dialog = screen.getByRole("dialog");
    expect(
      within(dialog).getByText(/revoke this source grant/),
    ).toBeInTheDocument();
    await userEvent.click(
      within(dialog).getByRole("button", {
        name: "Remove and revoke source grant",
      }),
    );
    await waitFor(() =>
      expect(mocks.revoke).toHaveBeenCalledWith({
        expected_grant_id: "grant",
        expected_revision: 7,
      }),
    );
    expect(mocks.save).not.toHaveBeenCalled();
  });
});
