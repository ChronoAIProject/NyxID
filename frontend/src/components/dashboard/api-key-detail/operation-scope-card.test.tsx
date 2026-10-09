import userEvent from "@testing-library/user-event";
import { render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { OperationScopeCard } from "./operation-scope-card";
import { ApiError } from "@/lib/api-client";
import type { AgentServiceOperations } from "@/schemas/agent-operation-scopes";

const { save, feature, query } = vi.hoisted(() => ({
  save: vi.fn(),
  feature: vi.fn(),
  query: vi.fn(),
}));
vi.mock("@/hooks/use-feature-flag", () => ({ useFeature: feature }));
vi.mock("@/hooks/use-api-key-operations", () => ({
  useApiKeyOperations: query,
  useSetApiKeyOperations: () => ({ mutateAsync: save, isPending: false }),
}));

const service: AgentServiceOperations = {
  service_id: "svc",
  service_slug: "calendar",
  service_name: "Calendar",
  revision: 0,
  allows_explicit_rules: false,
  all_operations: true,
  endpoint_ids: [],
  rules: [],
  operations: [
    {
      endpoint_id: "list",
      method: "GET",
      path: "/events",
      summary: "List events",
      read_only: true,
      changes_existing: false,
    },
    {
      endpoint_id: "delete",
      method: "DELETE",
      path: "/events/{id}",
      summary: "Delete event",
      read_only: false,
      changes_existing: true,
    },
  ],
};

beforeEach(() => {
  feature.mockReset();
  feature.mockReturnValue(true);
  query.mockReset();
  query.mockReturnValue({ data: [service] });
  save.mockReset();
  save.mockResolvedValue({ revision: 1 });
});

describe("Agent Key operation scopes", () => {
  it("renders nothing until operation scope configuration is enabled", () => {
    feature.mockReturnValue(false);
    const { container } = render(<OperationScopeCard keyId="key" canWrite />);
    expect(container).toBeEmptyDOMElement();
  });

  it("narrows a key's service to selected reads", async () => {
    const user = userEvent.setup();
    render(<OperationScopeCard keyId="key" canWrite />);
    await user.click(
      screen.getByRole("combobox", { name: "Calendar operation access" }),
    );
    await user.click(screen.getByRole("option", { name: "Selected operations" }));
    await user.click(screen.getByRole("button", { name: "Select all reads" }));
    await user.click(screen.getByRole("button", { name: "Save operations" }));
    await waitFor(() =>
      expect(save).toHaveBeenCalledWith({
        serviceId: "svc",
        selection: {
          expected_revision: 0,
          all_operations: false,
          endpoint_ids: ["list"],
          rules: [],
        },
      }),
    );
  });

  it("keeps saved value limits for operations that stay selected", async () => {
    const user = userEvent.setup();
    const limits = { query: { maxResults: { required: true, rule: { type: "max_integer", value: 50 } } } };
    query.mockReturnValue({
      data: [
        {
          ...service,
          all_operations: false,
          endpoint_ids: ["list", "delete"],
          inputs: { list: limits, delete: { path: {} } },
        },
      ],
    });
    render(<OperationScopeCard keyId="key" canWrite />);
    expect(screen.getAllByText("Value limits")).toHaveLength(2);
    await user.click(screen.getByRole("checkbox", { name: "DELETE /events/{id}" }));
    await user.click(screen.getByRole("button", { name: "Save operations" }));
    await waitFor(() =>
      expect(save).toHaveBeenCalledWith({
        serviceId: "svc",
        selection: {
          expected_revision: 0,
          all_operations: false,
          endpoint_ids: ["list"],
          rules: [],
          inputs: { list: limits },
        },
      }),
    );
  });

  it("is read-only for members without write access", () => {
    render(<OperationScopeCard keyId="key" canWrite={false} />);
    expect(
      screen.getByRole("combobox", { name: "Calendar operation access" }),
    ).toBeDisabled();
  });

  it("hides itself for keys the server cannot scope", () => {
    query.mockReturnValue({
      error: new ApiError(400, {
        error: "validation_error",
        error_code: 1008,
        message: "Operation scopes can only be set on ordinary Agent Keys",
      }),
    });
    const { container } = render(<OperationScopeCard keyId="key" canWrite />);
    expect(container).toBeEmptyDOMElement();
  });

  it("explains a key with no services", () => {
    query.mockReturnValue({ data: [] });
    render(<OperationScopeCard keyId="key" canWrite />);
    expect(
      screen.getByText("This key has no services to limit yet."),
    ).toBeInTheDocument();
  });
});
