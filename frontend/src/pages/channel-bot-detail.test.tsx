import { fireEvent, render, screen } from "@testing-library/react";
import { beforeEach, expect, it, vi } from "vitest";
import { AddRouteDialog } from "./channel-bot-detail";

const { refetch, apiKeys, queryState, create } = vi.hoisted(() => ({
  refetch: vi.fn(),
  apiKeys: vi.fn(),
  queryState: { error: null as Error | null },
  create: vi.fn(),
}));
vi.mock("@/hooks/use-api-keys", () => ({
  useApiKeys: (scope: unknown) => {
    apiKeys(scope);
    return { data: [], isLoading: false, error: queryState.error, refetch };
  },
}));
vi.mock("@/hooks/use-channel-conversations", () => ({
  useCreateChannelConversation: () => ({ mutate: create, isPending: false }),
}));
vi.mock("@/components/layout/dashboard-layout", () => ({
  useBreadcrumbLabel: vi.fn(),
}));

beforeEach(() => {
  vi.clearAllMocks();
  queryState.error = null;
});

it("loads the bot owner's keys and disables submission with no eligible keys", () => {
  render(
    <AddRouteDialog
      open
      onOpenChange={vi.fn()}
      botId="bot"
      ownerOrgId="org-1"
      ownerLabel="ChronoAI"
    />,
  );
  expect(apiKeys).toHaveBeenCalledWith({ orgId: "org-1" });
  expect(screen.getByRole("button", { name: "Add Route" })).toBeDisabled();
  expect(screen.getByText(/No agent keys exist/)).toBeInTheDocument();
  expect(create).not.toHaveBeenCalled();
});

it("has a working retry control when key loading fails", () => {
  queryState.error = new Error("offline");
  render(
    <AddRouteDialog
      open
      onOpenChange={vi.fn()}
      botId="bot"
      ownerOrgId="org-1"
      ownerLabel="ChronoAI"
    />,
  );
  fireEvent.click(screen.getByRole("button", { name: "Retry" }));
  expect(refetch).toHaveBeenCalledOnce();
  expect(screen.getByRole("button", { name: "Add Route" })).toBeDisabled();
});
