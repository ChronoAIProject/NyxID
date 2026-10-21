import { clearOnboardingLink } from "@/lib/platform-onboarding-handoff";
import { beforeEach, expect, it, vi } from "vitest";
import { render, screen, fireEvent } from "@testing-library/react";
import {
  OwnedOnboardingBindings,
  PlatformBindingReviewPage,
  PlatformSetupsPage,
} from "./platform-onboarding";
const { state, search, mutate, binding } = vi.hoisted(() => ({
  state: { user: { id: "actor-a", email: "a@example.test" } },
  search: { code: "ABCD1234", template: "restaurant-starter" },
  mutate: vi.fn(),
  binding: {
    id: "binding",
    generation: 1,
    manager_bot_id: "100",
    telegram_sender_id: "123",
    expires_at: "2026-10-15T00:00:00Z",
  },
}));
vi.mock("@/stores/auth-store", () => ({
  useAuthStore: (select: (s: typeof state) => unknown) => select(state),
}));
vi.mock("@tanstack/react-router", () => ({
  useRouterState: (input: {
    select: (s: {
      location: { search: typeof search; href: string };
    }) => unknown;
  }) => input.select({ location: { search, href: window.location.href } }),
  Link: ({ children }: { children: React.ReactNode }) => <a>{children}</a>,
}));
vi.mock("@/hooks/use-platform-onboarding", () => ({
  useBindingPreview: () => ({
    data: {
      request_id: "request",
      manager_bot_id: "100",
      requesting_sender_id: "123",
      state: "pending",
      expires_at: new Date(Date.now() + 600000).toISOString(),
      scope: "Onboarding only",
    },
  }),
  useBindingDecision: () => ({ mutate, isPending: false }),
  useArchivedSetups: () => ({
    data: { setups: [], next_before_id: null, next_before_ms: null },
  }),
  usePlatformSetups: () => ({ data: [] }),
  useOwnedBindings: () => ({ data: [{ ...binding }] }),
  useRevokeBinding: () => ({ mutate }),
  useSetupMutation: () => ({ mutate }),
}));
beforeEach(() => {
  state.user = { id: "actor-a", email: "a@example.test" };
  search.code = "ABCD1234";
  mutate.mockReset();
  sessionStorage.clear();
  clearOnboardingLink();
  window.history.replaceState(null, "", "/onboarding/link#code=ABCD1234");
});
it("resets consent when the signed-in account changes", () => {
  const view = render(<PlatformBindingReviewPage />);
  fireEvent.click(screen.getByRole("checkbox"));
  state.user = { id: "actor-b", email: "b@example.test" };
  view.rerender(<PlatformBindingReviewPage />);
  expect(screen.queryByRole("textbox")).not.toBeInTheDocument();
  expect(screen.getByRole("checkbox")).not.toBeChecked();
  expect(
    screen.getByRole("button", { name: "Approve account linking" }),
  ).toBeEnabled();
  expect(mutate).not.toHaveBeenCalled();
  expect(window.location.href).not.toContain("ABCD1234");
});
it("resets consent when the request code changes", () => {
  const view = render(<PlatformBindingReviewPage />);
  fireEvent.click(screen.getByRole("checkbox"));
  search.code = "EFGH5678";
  window.history.replaceState(null, "", "/onboarding/link#code=EFGH5678");
  view.rerender(<PlatformBindingReviewPage />);
  expect(screen.queryByRole("textbox")).not.toBeInTheDocument();
  expect(screen.getByRole("checkbox")).not.toBeChecked();
});
it("discards another account's local draft facts", () => {
  const view = render(<PlatformSetupsPage />);
  fireEvent.change(screen.getByLabelText("Assistant or restaurant name"), {
    target: { value: "Private draft" },
  });
  state.user = { id: "actor-b", email: "b@example.test" };
  view.rerender(<PlatformSetupsPage />);
  expect(screen.getByLabelText("Assistant or restaurant name")).toHaveValue("");
  expect(screen.getByRole("button", { name: "Save draft" })).toBeDisabled();
});
it("reuses the pending request after remount", async () => {
  const view = render(<PlatformSetupsPage />);
  fireEvent.change(screen.getByLabelText("Assistant or restaurant name"), {
    target: { value: "Cafe" },
  });
  fireEvent.submit(
    screen.getByRole("button", { name: "Save draft" }).closest("form")!,
  );
  await vi.waitFor(() => expect(mutate).toHaveBeenCalledTimes(1));
  const first = mutate.mock.calls[0]![0].request_id;
  view.unmount();
  render(<PlatformSetupsPage />);
  fireEvent.change(screen.getByLabelText("Assistant or restaurant name"), {
    target: { value: "Cafe" },
  });
  fireEvent.submit(
    screen.getByRole("button", { name: "Save draft" }).closest("form")!,
  );
  await vi.waitFor(() => expect(mutate).toHaveBeenCalledTimes(2));
  expect(mutate.mock.calls[1]![0].request_id).toBe(first);
});

it("requires rereview when the binding changes before revocation", () => {
  binding.generation = 1;
  const view = render(<OwnedOnboardingBindings />);
  fireEvent.click(screen.getByRole("button", { name: "Review revocation" }));
  binding.generation = 2;
  view.rerender(<OwnedOnboardingBindings />);
  expect(screen.getByRole("button", { name: "Revoke binding" })).toBeDisabled();
  fireEvent.click(
    screen.getByRole("button", { name: "Review current binding" }),
  );
  fireEvent.click(screen.getByRole("button", { name: "Revoke binding" }));
  expect(mutate.mock.calls[0]![0].generation).toBe(2);
});
