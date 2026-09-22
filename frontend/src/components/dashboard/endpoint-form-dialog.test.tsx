import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import type { ServiceEndpoint } from "@/types/api";
import { EndpointFormDialog } from "./endpoint-form-dialog";

vi.mock("sonner", () => ({
  toast: {
    info: vi.fn(),
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

    await user.clear(screen.getByLabelText("Path"));
    await user.type(screen.getByLabelText("Path"), "/people");
    await user.click(screen.getByRole("button", { name: "Save Changes" }));
    expect(onSubmit).not.toHaveBeenCalled();
    await user.click(
      await screen.findByRole("button", { name: "Confirm changes" }),
    );

    await waitFor(() => {
      expect(onSubmit).toHaveBeenCalledWith(
        {
          name: endpoint.name,
          description: existingDescription,
          method: "GET",
          path: "/people",
          parameters: "",
          request_body_schema: "",
          response_description: "",
        },
        { path: "/people" },
      );
    });
    expect(
      screen.queryByText("Description must be at most 500 characters"),
    ).not.toBeInTheDocument();
  });
});

it("treats JSON formatting as unchanged and preserves a draft on refetch", async () => {
  const user = userEvent.setup();
  const onSubmit = vi.fn();
  const saved = { ...endpoint, parameters: { limit: 10 } };
  const props = {
    open: true,
    onOpenChange: vi.fn(),
    endpoint: saved,
    onSubmit,
    isPending: false,
  };
  const view = render(<EndpointFormDialog {...props} />);
  const parameters = screen.getByLabelText(/Parameters/);
  fireEvent.change(parameters, { target: { value: '{"limit":10}' } });
  view.rerender(<EndpointFormDialog {...props} endpoint={{ ...saved }} />);
  expect(parameters).toHaveValue('{"limit":10}');
  await user.click(screen.getByRole("button", { name: "Save Changes" }));
  await waitFor(() =>
    expect(
      screen.queryByRole("dialog", { name: "Review changes" }),
    ).not.toBeInTheDocument(),
  );
  expect(onSubmit).not.toHaveBeenCalled();
});
