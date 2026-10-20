import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { QueryClient } from "@tanstack/react-query";
import { beforeEach, expect, it, vi } from "vitest";

const mocks = vi.hoisted(() => ({ buildId: "next", available: true, refresh: vi.fn() }));
vi.mock("@/hooks/use-build-updates", () => ({ useBuildUpdates: () => mocks }));
import { BuildUpdateBanner } from "./build-update-banner";

beforeEach(() => {
  mocks.buildId = "next";
  mocks.available = true;
  mocks.refresh.mockReset().mockResolvedValue(false);
});

it("shows a nonmodal banner for a new frontend build and refreshes only on request", async () => {
  render(<BuildUpdateBanner ready queryClient={new QueryClient()} />);
  expect(screen.getByRole("status")).toHaveTextContent("A new version of NyxID is ready.");
  expect(mocks.refresh).not.toHaveBeenCalled();
  fireEvent.click(screen.getByRole("button", { name: "Refresh" }));
  await waitFor(() => expect(mocks.refresh).toHaveBeenCalledOnce());
});

it("waits until the update is available before showing the banner", () => {
  mocks.available = false;
  const { rerender } = render(<BuildUpdateBanner ready queryClient={new QueryClient()} />);
  expect(screen.queryByRole("status")).toBeNull();
  mocks.available = true;
  rerender(<BuildUpdateBanner ready queryClient={new QueryClient()} />);
  expect(screen.getByRole("status")).toBeVisible();
});

it("lets a user dismiss the banner without changing the running app", () => {
  render(<BuildUpdateBanner ready queryClient={new QueryClient()} />);
  fireEvent.click(screen.getByRole("button", { name: "Dismiss update banner" }));
  expect(screen.queryByRole("status")).toBeNull();
  expect(mocks.refresh).not.toHaveBeenCalled();
});

it("shows a later release after an earlier release was dismissed", () => {
  const client = new QueryClient();
  const { rerender } = render(<BuildUpdateBanner ready queryClient={client} />);
  fireEvent.click(screen.getByRole("button", { name: "Dismiss update banner" }));
  expect(screen.queryByRole("status")).toBeNull();

  // Repeated observations must not undo dismissal for the same build.
  rerender(<BuildUpdateBanner ready queryClient={client} />);
  expect(screen.queryByRole("status")).toBeNull();

  mocks.buildId = "newer";
  rerender(<BuildUpdateBanner ready queryClient={client} />);
  expect(screen.getByRole("status")).toBeVisible();
  expect(mocks.refresh).not.toHaveBeenCalled();
});

it("does not carry a failed refresh message into a later release", async () => {
  const client = new QueryClient();
  const { rerender } = render(<BuildUpdateBanner ready queryClient={client} />);
  fireEvent.click(screen.getByRole("button", { name: "Refresh" }));
  await waitFor(() => expect(screen.getByRole("status")).toHaveTextContent("The update is still being deployed."));
  fireEvent.click(screen.getByRole("button", { name: "Dismiss update banner" }));

  mocks.buildId = "newer";
  rerender(<BuildUpdateBanner ready queryClient={client} />);
  expect(screen.getByRole("status")).toHaveTextContent("You can keep working.");
  expect(screen.getByRole("status")).not.toHaveTextContent("The update is still being deployed.");
});
