import { rememberBillingCheckout } from "@/lib/billing-checkout-return";
import { act, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { User } from "@/types/api";
import { BillingRouteGuard } from "./billing-route-guard";
import { useAuthStore } from "@/stores/auth-store";

const { navigate } = vi.hoisted(() => ({
  navigate: vi.fn(),
}));

vi.mock("@tanstack/react-router", () => ({
  useNavigate: () => navigate,
  useSearch: () => ({}),
}));

function testUser(billingAvailable: boolean): User {
  return {
    id: "user-1",
    email: "user@example.com",
    display_name: "Test User",
    avatar_url: null,
    email_verified: true,
    mfa_enabled: false,
    is_admin: false,
    is_active: true,
    created_at: "2026-01-01T00:00:00Z",
    capabilities: {
      billing_available: billingAvailable,
    },
  };
}

beforeEach(() => {
  navigate.mockReset();
  sessionStorage.clear();
  useAuthStore.setState({
    user: null,
    isAuthenticated: false,
    isLoading: true,
    mfaRequired: false,
    mfaToken: null,
  });
});

describe("BillingRouteGuard", () => {
    it("redirects after billing capability resolves unavailable", async () => {
      render(
        <BillingRouteGuard>
          <div>Billing content</div>
        </BillingRouteGuard>,
      );

      expect(screen.queryByText("Billing content")).not.toBeInTheDocument();
      expect(navigate).not.toHaveBeenCalled();

      act(() => {
        useAuthStore.setState({
          user: testUser(false),
          isAuthenticated: true,
          isLoading: false,
        });
      });

      await waitFor(() => {
        expect(navigate).toHaveBeenCalledWith({
          to: "/dashboard",
          replace: true,
        });
      });
      expect(screen.queryByText("Billing content")).not.toBeInTheDocument();
    });

    it("renders when billing capability is available", () => {
      act(() => {
        useAuthStore.setState({
          user: testUser(true),
          isAuthenticated: true,
          isLoading: false,
        });
      });

      render(
        <BillingRouteGuard>
          <div>Billing content</div>
        </BillingRouteGuard>,
      );

      expect(screen.getByText("Billing content")).toBeInTheDocument();
      expect(navigate).not.toHaveBeenCalled();
    });
});

it("redirects a signed-in assistant visitor with a missing billing capability", async () => {
  useAuthStore.setState({
    isAuthenticated: true,
    isLoading: false,
    user: { ...testUser(true), capabilities: {} },
  });
  render(
    <BillingRouteGuard>
      <div>Billing content</div>
    </BillingRouteGuard>,
  );
  await waitFor(() =>
    expect(navigate).toHaveBeenCalledWith({ to: "/dashboard", replace: true }),
  );
});

it("waits for authentication then restores org checkout before mounting the personal wallet", async () => {
  const actor = "11111111-1111-4111-8111-111111111111";
  const org = "22222222-2222-4222-8222-222222222222";
  rememberBillingCheckout(actor, org, "panel");
  render(<BillingRouteGuard><div>Billing content</div></BillingRouteGuard>);
  expect(navigate).not.toHaveBeenCalled();
  act(() => useAuthStore.setState({ user: { ...testUser(true), id: actor }, isAuthenticated: true, isLoading: false }));
  await waitFor(() => expect(navigate).toHaveBeenCalledWith({ to: "/assistant", search: { panel: "billing", panelOwnerId: org }, replace: true }));
  expect(screen.queryByText("Billing content")).not.toBeInTheDocument();
});
