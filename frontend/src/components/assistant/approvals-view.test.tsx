import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { ReactNode } from "react";
import { resetAssistantTransport } from "@/lib/assistant/transport";
import { ApprovalsView } from "./approvals-view";

vi.mock("@tanstack/react-router", () => ({
  Link: ({ children }: { children: ReactNode }) => <a href="#">{children}</a>,
}));

function createWrapper() {
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  });
  return function Wrapper({ children }: { children: ReactNode }) {
    return (
      <QueryClientProvider client={queryClient}>{children}</QueryClientProvider>
    );
  };
}

describe("ApprovalsView", () => {
  beforeEach(() => {
    resetAssistantTransport(() => Date.parse("2026-07-16T00:00:00.000Z"));
  });

  it("lists the pending approval with its conversation title", async () => {
    render(<ApprovalsView />, { wrapper: createWrapper() });
    expect(await screen.findByText("Waiting on you")).toBeInTheDocument();
    expect(screen.getByText("Failed Stripe payments digest")).toBeInTheDocument();
    expect(
      screen.getByRole("button", { name: /approve and send/i }),
    ).toBeInTheDocument();
  });

  it("renders the seeded decided approvals in the History section", async () => {
    render(<ApprovalsView />, { wrapper: createWrapper() });
    expect(await screen.findByText("History")).toBeInTheDocument();
    // Each entry renders twice: mobile card list + desktop table.
    expect(await screen.findAllByText("Approved")).toHaveLength(2);
    expect(screen.getAllByText("Denied")).toHaveLength(2);
    expect(screen.getAllByText("Expired")).toHaveLength(2);
    // History rows link back to their source conversations.
    expect(
      screen.getAllByText("Rotate GitHub deploy key").length,
    ).toBeGreaterThan(0);
    expect(
      screen.getAllByText("Weekly usage report").length,
    ).toBeGreaterThan(0);
    // Decision channel column reflects where the decision landed.
    expect(screen.getByText("Mobile")).toBeInTheDocument();
  });

  it("moves a decided approval into the History section", async () => {
    const user = userEvent.setup();
    render(<ApprovalsView />, { wrapper: createWrapper() });
    await user.click(
      await screen.findByRole("button", { name: /approve and send/i }),
    );
    await waitFor(() => {
      expect(screen.getByText("Nothing waiting on you")).toBeInTheDocument();
    });
    expect(screen.getByText("History")).toBeInTheDocument();
    // The freshly approved stripe entry joins the seeded github approval
    // (2 entries × mobile card + desktop row).
    expect(screen.getAllByText("Approved")).toHaveLength(4);
    expect(
      screen.getAllByText(
        "Post the drafted summary to #payments-oncall as the assistant bot.",
      ).length,
    ).toBeGreaterThan(0);
  });
});
