import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { CredentialAcceptPage } from "./credential-accept";

const { hooks, mockNavigate, routerState } = vi.hoisted(() => ({
  hooks: {
    node: {
      data: {
        capabilities: { remote_credential_crypto_v1: true },
      },
      isLoading: false,
      error: null as unknown,
    },
  },
  mockNavigate: vi.fn(),
  routerState: {
    params: { nodeId: "node-1", pendingId: "pending-1" },
    search: {} as { return_to?: string },
  },
}));

vi.mock("@tanstack/react-router", () => ({
  useNavigate: () => mockNavigate,
  useParams: () => routerState.params,
  useSearch: () => routerState.search,
}));

vi.mock("@/hooks/use-nodes", () => ({
  useNode: () => hooks.node,
}));

vi.mock("@/components/layout/dashboard-layout", () => ({
  useBreadcrumbLabel: () => {},
}));

vi.mock("sonner", () => ({
  toast: { success: vi.fn() },
}));

beforeEach(() => {
  vi.clearAllMocks();
  routerState.search = {};
});

afterEach(() => {
  vi.restoreAllMocks();
});

async function clickBack() {
  const user = userEvent.setup();
  render(<CredentialAcceptPage />);

  await user.click(screen.getByRole("button", { name: "Back" }));
}

describe("CredentialAcceptPage return_to redirect guard", () => {
  it("honors a normal relative return_to path", async () => {
    const assignSpy = vi
      .spyOn(window.location, "assign")
      .mockImplementation(() => undefined);
    routerState.search = { return_to: "/nodes/abc" };

    await clickBack();

    expect(assignSpy).toHaveBeenCalledWith("/nodes/abc");
    expect(mockNavigate).not.toHaveBeenCalled();
  });

  it.each(["//evil.example", "/\\evil.example", "https://evil.example"])(
    "falls back instead of assigning unsafe return_to %s",
    async (returnTo) => {
      const assignSpy = vi
        .spyOn(window.location, "assign")
        .mockImplementation(() => undefined);
      routerState.search = { return_to: returnTo };

      await clickBack();

      expect(assignSpy).not.toHaveBeenCalled();
      expect(mockNavigate).toHaveBeenCalledWith({
        to: "/nodes/$nodeId",
        params: { nodeId: "node-1" },
      });
    },
  );
});
