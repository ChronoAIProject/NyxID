import { fireEvent, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { expect, it, vi } from "vitest";
import { EndpointList } from "./endpoint-list";
const mock = vi.hoisted(() => ({
  update: vi.fn(),
  remove: vi.fn(),
  serviceId: "service-a",
}));
vi.mock("@/hooks/use-endpoints", () => ({
  useEndpoints: () => ({
    data: [
      {
        id: "endpoint",
        service_id: mock.serviceId,
        name: "get_items",
        method: "GET",
        path: "/items",
        description: null,
        parameters: null,
        request_body_schema: null,
        response_description: null,
        is_active: true,
      },
    ],
    isLoading: false,
  }),
  useCreateEndpoint: () => ({ mutateAsync: vi.fn(), isPending: false }),
  useUpdateEndpoint: () => ({ mutateAsync: mock.update, isPending: false }),
  useDeleteEndpoint: () => ({ mutateAsync: mock.remove, isPending: false }),
  useDiscoverEndpoints: () => ({ mutateAsync: vi.fn(), isPending: false }),
}));
it("cancels endpoint review on a parent service switch even if the endpoint ID is reused", async () => {
  const user = userEvent.setup();
  const view = render(
    <EndpointList serviceId="service-a" hasApiSpecUrl={false} />,
  );
  await user.click(screen.getByRole("button", { name: "Edit endpoint" }));
  fireEvent.change(screen.getByLabelText("Path"), {
    target: { value: "/draft" },
  });
  await user.click(screen.getByRole("button", { name: "Save Changes" }));
  await screen.findByRole("button", { name: "Confirm changes" });
  mock.serviceId = "service-b";
  view.rerender(<EndpointList serviceId="service-b" hasApiSpecUrl={false} />);
  expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
  expect(mock.update).not.toHaveBeenCalled();
  await user.click(screen.getByRole("button", { name: "Delete endpoint" }));
  expect(mock.remove).not.toHaveBeenCalled();
  view.rerender(<EndpointList serviceId="service-c" hasApiSpecUrl={false} />);
  expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
  expect(mock.remove).not.toHaveBeenCalled();
});
