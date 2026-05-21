import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

// Mode-B-adjacent CLI auth landing page (`/cli-auth`). The CLI opens this
// URL with `?port=…&state=…&client_ua=…`; the page mints a fresh access
// token via the cookie session and bounces it back to the CLI's loopback
// callback. These tests pin: the loading/invalid/authenticated branches,
// the unauthenticated → /login redirect (preserving return_to), and the
// happy-path token POST + 127.0.0.1 callback redirect.

const { mockNavigate, mockPost, searchState, storeState } = vi.hoisted(() => ({
  mockNavigate: vi.fn(),
  mockPost: vi.fn(),
  searchState: {
    value: {} as {
      port?: string;
      state?: string;
      client_ua?: string;
    },
  },
  storeState: { isAuthenticated: false, isLoading: false },
}));

vi.mock("@tanstack/react-router", () => ({
  useNavigate: () => mockNavigate,
  useSearch: () => searchState.value,
}));

vi.mock("@/stores/auth-store", () => ({
  useAuthStore: () => storeState,
}));

vi.mock("@/lib/api-client", () => ({
  api: { post: mockPost },
}));

import { CliAuthPage } from "./cli-auth";

let assignSpy: ReturnType<typeof vi.spyOn>;

beforeEach(() => {
  vi.clearAllMocks();
  searchState.value = {};
  storeState.isAuthenticated = false;
  storeState.isLoading = false;
  window.history.pushState({}, "", "/cli-auth");
  assignSpy = vi
    .spyOn(window.location, "assign")
    .mockImplementation(() => {});
});

afterEach(() => {
  assignSpy.mockRestore();
});

describe("CliAuthPage", () => {
  it("shows a skeleton (no redirect, no token POST) while auth is still loading", () => {
    storeState.isLoading = true;
    searchState.value = { port: "5555" };

    const { container } = render(<CliAuthPage />);

    // Skeleton render — none of the resolved-state copy is present.
    expect(container.querySelector('[class*="h-32"]')).toBeTruthy();
    expect(
      screen.queryByText(/Invalid CLI Auth Request/i),
    ).not.toBeInTheDocument();
    expect(
      screen.queryByText(/CLI Authentication/i),
    ).not.toBeInTheDocument();
    expect(assignSpy).not.toHaveBeenCalled();
    expect(mockPost).not.toHaveBeenCalled();
  });

  it("redirects unauthenticated users to /login with a return_to that preserves port/state/client_ua", async () => {
    storeState.isAuthenticated = false;
    searchState.value = { port: "5555", state: "xyz", client_ua: "nyxid/1.2" };

    render(<CliAuthPage />);

    await waitFor(() => {
      expect(assignSpy).toHaveBeenCalledTimes(1);
    });
    const target = assignSpy.mock.calls[0]![0] as string;
    expect(target.startsWith("/login?return_to=")).toBe(true);
    const returnTo = decodeURIComponent(target.split("return_to=")[1]!);
    expect(returnTo).toBe(
      `${window.location.origin}/cli-auth?port=5555&state=xyz&client_ua=nyxid%2F1.2`,
    );
    // No token request when not authenticated.
    expect(mockPost).not.toHaveBeenCalled();
  });

  it("renders the invalid-request panel and routes to /dashboard when there is no port", async () => {
    storeState.isAuthenticated = true;
    searchState.value = {}; // authenticated but missing the CLI callback port

    const user = userEvent.setup();
    render(<CliAuthPage />);

    expect(
      screen.getByRole("heading", { name: /Invalid CLI Auth Request/i }),
    ).toBeInTheDocument();
    // Authenticated-but-no-port must NOT attempt the token callback.
    expect(mockPost).not.toHaveBeenCalled();
    expect(assignSpy).not.toHaveBeenCalled();

    await user.click(screen.getByRole("button", { name: /Go to Dashboard/i }));
    expect(mockNavigate).toHaveBeenCalledWith({ to: "/dashboard" });
  });

  it("mints a CLI token and redirects to the 127.0.0.1 callback with state on the happy path", async () => {
    storeState.isAuthenticated = true;
    searchState.value = { port: "5555", state: "st-1", client_ua: "nyxid/9" };
    mockPost.mockResolvedValueOnce({
      access_token: "acc-123",
      refresh_token: "ref-456",
    });

    render(<CliAuthPage />);

    // The progress copy renders for an authenticated + port request.
    expect(
      screen.getByRole("heading", { name: /CLI Authentication/i }),
    ).toBeInTheDocument();

    await waitFor(() => {
      expect(mockPost).toHaveBeenCalledWith("/auth/cli-token", {
        client_ua: "nyxid/9",
      });
    });
    await waitFor(() => {
      expect(assignSpy).toHaveBeenCalledTimes(1);
    });
    const callback = new URL(assignSpy.mock.calls[0]![0] as string);
    expect(callback.origin).toBe("http://127.0.0.1:5555");
    expect(callback.pathname).toBe("/callback");
    expect(callback.searchParams.get("access_token")).toBe("acc-123");
    expect(callback.searchParams.get("refresh_token")).toBe("ref-456");
    expect(callback.searchParams.get("state")).toBe("st-1");
  });

  it("omits state from the callback URL when the CLI did not supply one", async () => {
    storeState.isAuthenticated = true;
    searchState.value = { port: "6000" };
    mockPost.mockResolvedValueOnce({
      access_token: "a",
      refresh_token: "r",
    });

    render(<CliAuthPage />);

    await waitFor(() => {
      expect(assignSpy).toHaveBeenCalledTimes(1);
    });
    const callback = new URL(assignSpy.mock.calls[0]![0] as string);
    expect(callback.searchParams.has("state")).toBe(false);
    // client_ua is undefined → still POSTs with the undefined value.
    expect(mockPost).toHaveBeenCalledWith("/auth/cli-token", {
      client_ua: undefined,
    });
  });

  it("renders an in-page failure message (no callback redirect) when token minting fails", async () => {
    storeState.isAuthenticated = true;
    searchState.value = { port: "7000" };
    mockPost.mockRejectedValueOnce(new Error("session expired"));

    render(<CliAuthPage />);

    await waitFor(() => {
      expect(mockPost).toHaveBeenCalledTimes(1);
    });
    // The catch path rewrites document.body with a failure notice and
    // never redirects to the loopback callback.
    await waitFor(() => {
      expect(document.body.innerHTML).toMatch(
        /Failed to send token to CLI/i,
      );
    });
    expect(assignSpy).not.toHaveBeenCalled();
  });
});
