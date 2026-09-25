import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, expect, it, vi } from "vitest";
import { CreateServiceDialog } from "./create-service-dialog";

const { create, navigate } = vi.hoisted(() => ({
  create: vi.fn(),
  navigate: vi.fn(),
}));
vi.mock("@/hooks/use-services", () => ({
  useCreateService: () => ({ mutateAsync: create, isPending: false }),
}));
vi.mock("@tanstack/react-router", () => ({ useNavigate: () => navigate }));
vi.mock("@/components/admin-credits/credit-pickers", () => ({
  UserPicker: () => <span>Owner picker</span>,
}));
vi.mock("sonner", () => ({ toast: { success: vi.fn(), error: vi.fn() } }));

beforeEach(() => {
  create.mockReset().mockResolvedValue({ id: "new-service" });
});

it("creates a provider-linked service with the shared configuration and disabled restricted defaults", async () => {
  const user = userEvent.setup();
  render(
    <CreateServiceDialog
      open
      onOpenChange={vi.fn()}
      provider={{ id: "telnyx", name: "Telnyx" }}
    />,
  );
  await user.type(screen.getByLabelText("Service Name"), "Shared calls");
  await user.type(
    screen.getByLabelText("Base URL"),
    "https://api.telnyx.com/v2",
  );
  expect(
    screen.getByRole("switch", { name: "Enable platform key" }),
  ).not.toBeChecked();
  expect(screen.getByText("Owner picker")).toBeInTheDocument();
  expect(
    screen.getByText("Shared credential: not configured"),
  ).toBeInTheDocument();
  await user.type(
    screen.getByLabelText("Shared platform credential"),
    "fixture-value",
  );
  await user.click(
    screen.getByRole("switch", { name: "Restrict allowed endpoints" }),
  );
  await user.click(screen.getByRole("button", { name: "Add endpoint rule" }));
  await user.clear(screen.getByLabelText("Rule 1 path"));
  await user.type(screen.getByLabelText("Rule 1 path"), "/ai/openai/models");
  await user.click(screen.getByRole("button", { name: "Create service" }));
  await waitFor(() => expect(create).toHaveBeenCalledOnce());
  expect(create.mock.calls[0]?.[0]).toMatchObject({
    provider_config_id: "telnyx",
    credential: "fixture-value",
    service_category: "connection",
    platform_key: {
      enabled: false,
      audience: "restricted",
      allowed_owner_ids: [],
    },
    proxy_operation_policy: {
      rules: [{ method: "GET", path_template: "/ai/openai/models" }],
    },
  });
});
