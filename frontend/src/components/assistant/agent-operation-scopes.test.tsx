import {
  fireEvent,
  render,
  screen,
  waitFor,
  within,
} from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { AgentOperationScopes, ServiceOperationForm } from "./agent-operation-scopes";
import type { AgentServiceOperations } from "@/schemas/agent-operation-scopes";

const { save, feature, query } = vi.hoisted(() => ({ save: vi.fn(), feature: vi.fn(), query: vi.fn() }));
vi.mock("@/hooks/use-feature-flag", () => ({ useFeature: feature }));
vi.mock("@/hooks/use-agent-operation-scopes", () => ({
  useSetAgentOperations: () => ({ mutateAsync: save, isPending: false }),
  useAgentOperations: query,
}));
const service: AgentServiceOperations = {
  service_id: "svc",
  service_slug: "example",
  service_name: "Example",
  revision: 4,
  allows_explicit_rules: false,
  all_operations: false,
  endpoint_ids: ["write"],
  rules: [],
  operations: [
    {
      endpoint_id: "read",
      method: "GET",
      path: "/items",
      summary: "List items",
      read_only: true,
      changes_existing: false,
    },
    {
      endpoint_id: "write",
      method: "DELETE",
      path: "/items/{id}",
      summary: "Delete item",
      read_only: false,
      changes_existing: true,
    },
    {
      endpoint_id: "marked",
      method: "POST",
      path: "/search",
      summary: "Search items",
      read_only: true,
      changes_existing: false,
    },
  ],
};
beforeEach(() => {
  feature.mockReset();
  feature.mockReturnValue(false);
  query.mockReset();
  query.mockReturnValue({ data: [service] });
  save.mockReset();
  save.mockResolvedValue({ revision: 5 });
});
describe("agent operation scopes", () => {
  it("hides configuration and skips catalog queries while the rollout flag is off", () => {
    render(<AgentOperationScopes agentId="agent" />);
    expect(feature).toHaveBeenCalledWith("assistant:operation-scopes");
    expect(query).toHaveBeenCalledWith("agent", false);
    expect(screen.getByRole("status")).toHaveTextContent("not enabled yet");
    expect(screen.getByRole("status")).toHaveTextContent("Existing operation limits still apply");
    expect(screen.queryByRole("combobox")).not.toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Save operations" })).not.toBeInTheDocument();
  });
  it("shows configuration when the rollout flag is enabled", () => {
    feature.mockReturnValue(true);
    render(<AgentOperationScopes agentId="agent" />);
    expect(query).toHaveBeenCalledWith("agent", true);
    expect(screen.getByRole("combobox", { name: "Example operation access" })).toBeInTheDocument();
  });
  it("hides an already loaded selector when the flag is disabled", () => {
    feature.mockReturnValue(true);
    const view = render(<AgentOperationScopes agentId="agent" />);
    feature.mockReturnValue(false);
    view.rerender(<AgentOperationScopes agentId="agent" />);
    expect(query).toHaveBeenLastCalledWith("agent", false);
    expect(screen.queryByRole("combobox")).not.toBeInTheDocument();
    expect(screen.getByRole("status")).toHaveTextContent("not enabled yet");
  });
  it("selects catalog reads, excludes changes, and sends the reviewed revision", async () => {
    render(<ServiceOperationForm agentId="agent" service={service} />);
    expect(
      screen.getByRole("button", { name: "Save operations" }),
    ).toBeDisabled();
    fireEvent.click(screen.getByRole("button", { name: "Select all reads" }));
    expect(screen.getByRole("checkbox", { name: "GET /items" })).toBeChecked();
    expect(
      screen.getByRole("checkbox", { name: "POST /search" }),
    ).toBeChecked();
    expect(
      screen.getByRole("checkbox", { name: "DELETE /items/{id}" }),
    ).not.toBeChecked();
    fireEvent.click(screen.getByRole("button", { name: "Save operations" }));
    await waitFor(() =>
      expect(save).toHaveBeenCalledWith({
        serviceId: "svc",
        selection: {
          expected_revision: 4,
          all_operations: false,
          endpoint_ids: ["read", "marked"],
          rules: [],
        },
      }),
    );
  });
  it("searches without discarding selections and allows an empty deny selection", async () => {
    render(<ServiceOperationForm agentId="agent" service={service} />);
    fireEvent.change(
      screen.getByRole("textbox", { name: "Search Example operations" }),
      { target: { value: "DELETE" } },
    );
    expect(
      screen.queryByRole("checkbox", { name: "GET /items" }),
    ).not.toBeInTheDocument();
    fireEvent.click(
      screen.getByRole("checkbox", { name: "DELETE /items/{id}" }),
    );
    fireEvent.click(screen.getByRole("button", { name: "Save operations" }));
    await waitFor(() =>
      expect(save).toHaveBeenCalledWith(
        expect.objectContaining({
          selection: expect.objectContaining({
            endpoint_ids: [],
            all_operations: false,
          }),
        }),
      ),
    );
  });
  it("surfaces revision conflicts and retains the proposed selection", async () => {
    save.mockRejectedValue(
      new Error("Operation scope changed; reload its revision"),
    );
    render(<ServiceOperationForm agentId="agent" service={service} />);
    fireEvent.click(screen.getByRole("checkbox", { name: "GET /items" }));
    fireEvent.click(screen.getByRole("button", { name: "Save operations" }));
    expect(await screen.findByRole("alert")).toHaveTextContent(
      "reload its revision",
    );
    expect(screen.getByRole("checkbox", { name: "GET /items" })).toBeChecked();
  });
  it("edits explicit rules only when there are no catalog operations", async () => {
    render(
      <ServiceOperationForm
        agentId="agent"
        service={{
          ...service,
          operations: [],
          endpoint_ids: [],
          allows_explicit_rules: true,
        }}
      />,
    );
    fireEvent.click(screen.getByRole("button", { name: "Add rule" }));
    fireEvent.change(screen.getByRole("textbox", { name: "Rule 1 path" }), {
      target: { value: "/status/{id}" },
    });
    fireEvent.click(screen.getByRole("button", { name: "Save operations" }));
    await waitFor(() =>
      expect(save).toHaveBeenCalledWith(
        expect.objectContaining({
          selection: expect.objectContaining({
            rules: [{ method: "GET", path_template: "/status/{id}" }],
          }),
        }),
      ),
    );
  });
  it("prevents editing destroyed agents", () => {
    render(<ServiceOperationForm agentId="agent" service={service} disabled />);
    for (const checkbox of within(screen.getByRole("form")).getAllByRole(
      "checkbox",
    ))
      expect(checkbox).toBeDisabled();
    expect(
      screen.getByRole("button", { name: "Save operations" }),
    ).toBeDisabled();
  });
});
