import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { MachineGrantEditor } from "./machine-grant-editor";

const nodeId = "f3447880-a63f-4569-841b-389a87a7e4e3";
const mocks = vi.hoisted(() => ({ nodes: [] as unknown[], submit: vi.fn() }));
vi.mock("@/hooks/use-nodes", () => ({ useNodes: () => ({ data: mocks.nodes, isLoading: false, error: null }) }));
vi.mock("@/hooks/use-user-services", () => ({ useUserServices: () => ({ data: [], isLoading: false, error: null }) }));
vi.mock("@/stores/auth-store", () => ({ useAuthStore: (selector: (state: { user: { id: string } }) => unknown) => selector({ user: { id: "owner" } }) }));

beforeEach(() => {
  mocks.submit.mockReset().mockResolvedValue(undefined);
  mocks.nodes = [{
    id: nodeId,
    name: "Personal self-hosted Node",
    owner: { kind: "user", id: "owner" },
    machine: { shell: true, files: true, roots: ["/workspace"] },
    machine_confirm: "changes",
  }];
});

describe("actual shared Machine grant editor disclosures", () => {
  it("warns about OS access to Node identity and stored credentials on self-hosted shell grants", () => {
    render(<MachineGrantEditor onSubmit={mocks.submit} />);
    expect(screen.getByRole("group", { name: "Personal self-hosted Node" })).toBeInTheDocument();
    const warning = screen.getByRole("note", { name: "Shell access and credentials" });
    expect(warning).toHaveTextContent(/non-isolated shell/i);
    expect(warning).toHaveTextContent(/Node tokens/i);
    expect(warning).toHaveTextContent(/signing secrets/i);
    expect(warning).toHaveTextContent(/stored credentials/i);
    expect(warning).toHaveTextContent(/OS permissions/i);
    expect(warning).toHaveTextContent(/container/i);
    expect(warning).toHaveTextContent(/--separate-users/);
  });

  it("keeps the filesystem warning while excluded tool capabilities remain excluded on submission", async () => {
    render(<MachineGrantEditor onSubmit={mocks.submit} />);
    await userEvent.click(screen.getByRole("checkbox", { name: "Allow this Machine" }));
    expect(screen.getByRole("note", { name: "Shell access and credentials" })).toHaveTextContent(/excluding these tools does not protect their underlying files/i);
    await userEvent.click(screen.getByRole("checkbox", { name: /I explicitly allow shared legacy/ }));
    await userEvent.click(screen.getByRole("button", { name: "Save Machine permissions" }));
    await waitFor(() => expect(mocks.submit).toHaveBeenCalledWith(expect.objectContaining({
      assignments: { [nodeId]: {
        capabilities: { shell: true, files: true, browser: false, computer: false, developer_browser: false },
        file_roots: ["/workspace"], service_ids: [],
      } },
    }), expect.anything()));
  });

  it("distinguishes managed supervisor protection from isolation between clients", () => {
    mocks.nodes = [{
      id: nodeId, name: "Managed Machine", owner: { kind: "user", id: "owner" },
      machine: { shell: true, files: true, roots: ["/workspace"] }, machine_confirm: "changes",
    }];
    render(<MachineGrantEditor onSubmit={mocks.submit} />);
    const warning = screen.getByRole("note", { name: "Shell access and credentials" });
    expect(warning).toHaveTextContent(/managed Machines/i);
    expect(warning).toHaveTextContent(/supervisor.*command-user separation/i);
    expect(warning).toHaveTextContent(/protected mounts/i);
    expect(warning).toHaveTextContent(/does not isolate clients or their workspace secrets/i);
    expect(warning).toHaveTextContent(/file roots.*not what shell commands can access/i);
  });
});

it("never preselects legacy consent, including while editing an existing grant", async () => {
  render(<MachineGrantEditor initial={{ separation: "shared_legacy", legacy_warning_acknowledged: true, assignments: { [nodeId]: { capabilities: { shell: true, files: true, browser: false, computer: false, developer_browser: false }, file_roots: ["/workspace"], service_ids: [] } } }} onSubmit={mocks.submit} />);
  expect(screen.getByRole("checkbox", { name: /I explicitly allow shared legacy/ })).not.toBeChecked();
  await userEvent.click(screen.getByRole("button", { name: "Save Machine permissions" }));
  expect(await screen.findByText(/External Machine access requires explicit shared legacy consent/)).toBeInTheDocument();
  expect(mocks.submit).not.toHaveBeenCalled();
  const warning = screen.getByRole("note", { name: "Shell access and credentials" });
  expect(warning).toHaveTextContent(/share the host kernel/);
  expect(warning).toHaveTextContent(/separate machine container or VM per agent/);
});
