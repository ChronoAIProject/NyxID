import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, expect, it, vi } from "vitest";

const mocks = vi.hoisted(() => ({ rollout: vi.fn(), capability: vi.fn() }));
vi.mock("@/hooks/use-app-requirements", () => ({
  useAppConnectRollout: () => ({
    data: {
      effective: "disabled",
      env_default: "disabled",
      override_value: null,
      allowed_org_ids: [],
    },
    isPending: false,
  }),
  useUpdateAppConnectRollout: () => ({
    mutateAsync: mocks.rollout,
    isPending: false,
  }),
  useUpdateAppConnectCapability: () => ({
    mutateAsync: mocks.capability,
    isPending: false,
  }),
}));
vi.mock("sonner", () => ({ toast: { success: vi.fn(), error: vi.fn() } }));
import {
  AppConnectRolloutPolicy,
  AppConnectCapabilitySwitch,
} from "./app-connect-rollout";

beforeEach(() => {
  vi.clearAllMocks();
  mocks.rollout.mockResolvedValue({});
  mocks.capability.mockResolvedValue({});
});

it("offers allowlist rollout without a public-mode action", async () => {
  const user = userEvent.setup();
  render(<AppConnectRolloutPolicy />);
  expect(
    screen.queryByRole("button", { name: /public/i }),
  ).not.toBeInTheDocument();
  await user.click(screen.getByRole("button", { name: "Use allowlist" }));
  await waitFor(() => expect(mocks.rollout).toHaveBeenCalledWith("allowlist"));
});

it("updates the named client's capability through the admin mutation", async () => {
  const user = userEvent.setup();
  render(
    <AppConnectCapabilitySwitch
      clientId="app"
      clientName="Test App"
      enabled={false}
    />,
  );
  await user.click(
    screen.getByRole("switch", {
      name: "App Connect capability for Test App (app)",
    }),
  );
  await waitFor(() =>
    expect(mocks.capability).toHaveBeenCalledWith({
      clientId: "app",
      enabled: true,
    }),
  );
});
