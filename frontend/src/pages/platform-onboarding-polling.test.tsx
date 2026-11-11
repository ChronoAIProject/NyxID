import { cleanup, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { afterEach, expect, it, vi } from "vitest";
import { PlatformBindingReviewPage } from "./platform-onboarding";
import { apiClient } from "@/lib/api-client";
import { useAuthStore } from "@/stores/auth-store";
import { clearOnboardingLink } from "@/lib/platform-onboarding-handoff";

vi.mock("@/lib/api-client", async (original) => ({
  ...(await original<typeof import("@/lib/api-client")>()),
  apiClient: vi.fn(),
}));
vi.mock("@tanstack/react-router", () => ({
  useRouterState: ({ select }: { select: (state: unknown) => unknown }) =>
    select({ location: { href: window.location.href } }),
  Link: ({ children, to }: { children: React.ReactNode; to: string }) => (
    <a href={to}>{children}</a>
  ),
}));
let client: QueryClient;
afterEach(() => {
  cleanup();
  client?.clear();
  clearOnboardingLink();
  vi.mocked(apiClient).mockReset();
});

it("opens Telegram in a new tab and polls onboarding confirmation in the current page", async () => {
  const user = userEvent.setup();
  clearOnboardingLink();
  window.history.replaceState(null, "", "/onboarding/link#code=ABCD1234");
  useAuthStore.setState({
    user: { id: "owner", email: "owner@example.test" } as never,
  });
  let approved = false;
  let confirmed = false;
  let polls = 0;
  vi.mocked(apiClient).mockImplementation(async (path) => {
    if (path === "/onboarding/link/approve") approved = true;
    else if (path === "/onboarding/link/preview") polls += 1;
    else throw new Error(`Unexpected route: ${path}`);
    return {
      request_id: "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa",
      manager_bot_id: "100",
      manager_username: "manager_bot",
      requesting_sender_id: "700",
      state: confirmed
        ? "bound"
        : approved
          ? "awaiting_chat_confirmation"
          : "pending",
      expires_at: new Date(Date.now() + 600000).toISOString(),
      scope: "Onboarding only",
      confirmation_command: path.endsWith("approve")
        ? `/confirm O-${"A".repeat(32)}`
        : null,
    };
  });
  client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  render(
    <QueryClientProvider client={client}>
      <PlatformBindingReviewPage />
    </QueryClientProvider>,
  );
  await user.click(
    await screen.findByRole("button", { name: "Approve account linking" }),
  );
  const link = await screen.findByRole("link", {
    name: "Open Telegram to finish",
  });
  expect(link).toHaveAttribute("target", "_blank");
  expect(link).toHaveAttribute("rel", "noopener noreferrer");
  expect(link).toHaveAttribute(
    "href",
    `https://t.me/manager_bot?start=nyxonboard_O-${"A".repeat(32)}`,
  );
  const previousPolls = polls;
  confirmed = true;
  expect(
    await screen.findByText("Telegram account linked.", {}, { timeout: 6000 }),
  ).toBeVisible();
  expect(polls).toBeGreaterThan(previousPolls);
  expect(window.location.pathname).toBe("/onboarding/link");
}, 15000);
