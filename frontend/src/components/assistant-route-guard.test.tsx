import { act, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { User } from "@/types/api";
import { FEATURE_FLAG } from "@/lib/feature-flags";
import { AssistantRouteGuard } from "./assistant-route-guard";
import { useAuthStore } from "@/stores/auth-store";

const { navigate } = vi.hoisted(() => ({
  navigate: vi.fn(),
}));

vi.mock("@tanstack/react-router", () => ({
  useNavigate: () => navigate,
}));

function testUser(assistantAvailable: boolean): User {
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
      enabled_features: assistantAvailable ? [FEATURE_FLAG.AI_ASSISTANT] : [],
    },
  };
}

beforeEach(() => {
  navigate.mockReset();
  useAuthStore.setState({
    user: null,
    isAuthenticated: false,
    isLoading: true,
    mfaRequired: false,
    mfaToken: null,
  });
});

describe("AssistantRouteGuard", () => {
  it("redirects after the feature capability resolves unavailable", async () => {
    render(
      <AssistantRouteGuard>
        <div>Assistant content</div>
      </AssistantRouteGuard>,
    );

    expect(screen.queryByText("Assistant content")).not.toBeInTheDocument();
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
    expect(screen.queryByText("Assistant content")).not.toBeInTheDocument();
  });

  it("renders when the feature capability is available", () => {
    act(() => {
      useAuthStore.setState({
        user: testUser(true),
        isAuthenticated: true,
        isLoading: false,
      });
    });

    render(
      <AssistantRouteGuard>
        <div>Assistant content</div>
      </AssistantRouteGuard>,
    );

    expect(screen.getByText("Assistant content")).toBeInTheDocument();
    expect(navigate).not.toHaveBeenCalled();
  });
});
