import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import type { ServiceEndpoint } from "@/types/api";
import { EndpointFormDialog } from "./endpoint-form-dialog";

vi.mock("sonner", () => ({
  toast: {
    error: vi.fn(),
  },
}));

const existingDescription = "a".repeat(501);

const endpoint: ServiceEndpoint = {
  id: "endpoint-1",
  service_id: "service-1",
  name: "get_users",
  description: existingDescription,
  method: "GET",
  path: "/users",
  parameters: null,
  request_body_schema: null,
  response_description: null,
  is_active: true,
  created_at: "2026-03-19T00:00:00Z",
  updated_at: "2026-03-19T00:00:00Z",
};

describe("EndpointFormDialog", () => {
  it("submits an unchanged existing description even when it exceeds the new limit", async () => {
    const user = userEvent.setup();
    const onSubmit = vi.fn().mockResolvedValue(undefined);

    render(
      <EndpointFormDialog
        open
        onOpenChange={vi.fn()}
        endpoint={endpoint}
        onSubmit={onSubmit}
        isPending={false}
      />,
    );

    await user.click(screen.getByRole("button", { name: "Save changes" }));

    await waitFor(() => {
      expect(onSubmit).toHaveBeenCalledWith({
        name: endpoint.name,
        description: existingDescription,
        method: "GET",
        path: endpoint.path,
        parameters: "",
        request_body_schema: "",
        response_description: "",
      });
    });
    expect(
      screen.queryByText("Description must be at most 500 characters"),
    ).not.toBeInTheDocument();
  });
});
