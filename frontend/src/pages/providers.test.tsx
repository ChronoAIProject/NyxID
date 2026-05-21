import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

const { mockNavigate, llmStatusState } = vi.hoisted(() => ({
  mockNavigate: vi.fn(),
  // Holds the value returned by useLlmStatus().data. `undefined` means the
  // query hasn't resolved yet, which is the gate ProvidersPage checks.
  llmStatusState: { data: undefined as unknown },
}));

vi.mock("@tanstack/react-router", () => ({
  useNavigate: () => mockNavigate,
}));

vi.mock("@/hooks/use-llm-gateway", () => ({
  useLlmStatus: () => ({ data: llmStatusState.data }),
}));

// Stub the heavy child components — ProvidersPage is a composition shell, so
// we only assert that it mounts them and wires the gateway gate correctly.
vi.mock("@/components/dashboard/provider-grid", () => ({
  ProviderGrid: () => <div data-testid="provider-grid" />,
}));

vi.mock("@/components/dashboard/gateway-info-card", () => ({
  GatewayInfoCard: ({ llmStatus }: { readonly llmStatus: unknown }) => (
    <div data-testid="gateway-info-card">{JSON.stringify(llmStatus)}</div>
  ),
}));

import { ProvidersPage } from "./providers";

describe("ProvidersPage", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    llmStatusState.data = undefined;
  });

  it("renders the page header and always mounts the provider grid", () => {
    render(<ProvidersPage />);

    expect(
      screen.getByRole("heading", { name: "Providers" }),
    ).toBeInTheDocument();
    expect(screen.getByTestId("provider-grid")).toBeInTheDocument();
  });

  it("navigates to the manage page when the Manage Providers action is clicked", async () => {
    const user = userEvent.setup();
    render(<ProvidersPage />);

    await user.click(screen.getByRole("button", { name: /manage providers/i }));

    expect(mockNavigate).toHaveBeenCalledWith({ to: "/providers/manage" });
  });

  it("hides the gateway info card while llm status is undefined", () => {
    llmStatusState.data = undefined;

    render(<ProvidersPage />);

    expect(screen.queryByTestId("gateway-info-card")).not.toBeInTheDocument();
  });

  it("renders the gateway info card once llm status resolves", () => {
    const llmStatus = { gateway_url: "https://gw", providers: [] };
    llmStatusState.data = llmStatus;

    render(<ProvidersPage />);

    const card = screen.getByTestId("gateway-info-card");
    expect(card).toBeInTheDocument();
    // The resolved status is forwarded to the card unchanged.
    expect(card).toHaveTextContent(JSON.stringify(llmStatus));
  });
});
