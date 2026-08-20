import { render, screen } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { PreviewAuthDeviceResponse } from "@/schemas/auth-device";
import { LoginDevicePage } from "./login-device";

const { navigate, previewState } = vi.hoisted(() => ({
  navigate: vi.fn(),
  previewState: {
    data: undefined as PreviewAuthDeviceResponse | undefined,
  },
}));

vi.mock("@tanstack/react-router", () => ({
  Link: ({ children }: { readonly children: React.ReactNode }) => (
    <a href="/">{children}</a>
  ),
  useNavigate: () => navigate,
}));

vi.mock("@/stores/auth-store", () => ({
  useAuthStore: () => ({ isAuthenticated: true, isLoading: false }),
}));

vi.mock("@/hooks/use-auth-device", () => ({
  usePreviewAuthDevice: () => ({
    data: previewState.data,
    error: null,
    isError: false,
    isPending: false,
    mutateAsync: vi.fn(),
    reset: vi.fn(),
  }),
  useApproveAuthDevice: () => ({
    isPending: false,
    mutateAsync: vi.fn(),
    reset: vi.fn(),
  }),
  useDenyAuthDevice: () => ({
    isPending: false,
    mutateAsync: vi.fn(),
    reset: vi.fn(),
  }),
}));

beforeEach(() => {
  vi.clearAllMocks();
  previewState.data = undefined;
});

describe("LoginDevicePage", () => {
  it("offers decisions for a pending preview", () => {
    previewState.data = {
      client_label: "workstation",
      client_user_agent: "nyxid-cli",
      client_ip: null,
      initiated_at: "2026-08-20T10:00:00Z",
      expires_at: "2026-08-20T10:10:00Z",
      status: "pending",
    };

    render(<LoginDevicePage />);

    expect(
      screen.getByRole("button", { name: "Approve" }),
    ).toBeInTheDocument();
    expect(
      screen.getByRole("button", { name: "Reject" }),
    ).toBeInTheDocument();
  });

  it.each([
    ["denied", "This login request was already denied."],
    ["expired", "This code has expired."],
    ["approved", "This code was already used."],
    ["delivered", "This code was already used."],
  ] as const)(
    "does not offer decisions for a %s preview",
    (status, expectedMessage) => {
      previewState.data = {
        client_label: "workstation",
        client_user_agent: "nyxid-cli",
        client_ip: null,
        initiated_at: "2026-08-20T10:00:00Z",
        expires_at: "2026-08-20T10:10:00Z",
        status,
      };

      render(<LoginDevicePage />);

      expect(screen.getByText(expectedMessage)).toBeInTheDocument();
      expect(
        screen.queryByRole("button", { name: "Approve" }),
      ).not.toBeInTheDocument();
      expect(
        screen.queryByRole("button", { name: "Reject" }),
      ).not.toBeInTheDocument();
    },
  );
});
