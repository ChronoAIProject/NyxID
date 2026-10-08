import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { QueryClient } from "@tanstack/react-query";
import { beforeEach, expect, it, vi } from "vitest";

const mocks = vi.hoisted(() => ({ path: "/assistant", available: true, assistantChanged: true, refresh: vi.fn() }));
vi.mock("@/router", () => ({ router: {} }));
vi.mock("@tanstack/react-router", () => ({ useRouterState: () => mocks.path }));
vi.mock("@/hooks/use-build-updates", () => ({ useBuildUpdates: () => mocks }));
import { BuildUpdateBanner } from "./build-update-banner";

beforeEach(() => {
  mocks.path = "/assistant";
  mocks.available = true;
  mocks.assistantChanged = true;
  mocks.refresh.mockReset().mockResolvedValue(false);
});

it("shows a nonmodal banner for changed assistant code and refreshes only on request", async () => {
  render(<BuildUpdateBanner ready queryClient={new QueryClient()} />);
  expect(screen.getByRole("status")).toHaveTextContent("A new version of NyxID is ready.");
  expect(mocks.refresh).not.toHaveBeenCalled();
  fireEvent.click(screen.getByRole("button", { name: "Refresh" }));
  await waitFor(() => expect(mocks.refresh).toHaveBeenCalledOnce());
});

it("suppresses unrelated deployments over assistant pages while retaining the banner elsewhere", () => {
  mocks.assistantChanged = false;
  const { rerender } = render(<BuildUpdateBanner ready queryClient={new QueryClient()} />);
  expect(screen.queryByRole("status")).toBeNull();
  mocks.path = "/dashboard";
  rerender(<BuildUpdateBanner ready queryClient={new QueryClient()} />);
  expect(screen.getByRole("status")).toBeVisible();
});

it("lets a user dismiss the banner without changing the running app", () => {
  render(<BuildUpdateBanner ready queryClient={new QueryClient()} />);
  fireEvent.click(screen.getByRole("button", { name: "Dismiss update banner" }));
  expect(screen.queryByRole("status")).toBeNull();
  expect(mocks.refresh).not.toHaveBeenCalled();
});
