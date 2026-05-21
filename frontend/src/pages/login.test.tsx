import { render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { LoginPage } from "./login";

const { storeState } = vi.hoisted(() => ({
  storeState: { mfaRequired: false },
}));

// Stub children so we can assert WHICH one renders and WITH WHAT props.
vi.mock("@/components/auth/auth-flow", () => ({
  AuthFlow: (props: Record<string, unknown>) => (
    <div data-testid="auth-flow" data-props={JSON.stringify(props)} />
  ),
}));

vi.mock("@/components/auth/mfa-verify-form", () => ({
  MfaVerifyForm: (props: Record<string, unknown>) => (
    <div data-testid="mfa-verify-form" data-props={JSON.stringify(props)} />
  ),
}));

vi.mock("@/stores/auth-store", () => ({
  useAuthStore: (selector: (s: { mfaRequired: boolean }) => unknown) =>
    selector({ mfaRequired: storeState.mfaRequired }),
}));

beforeEach(() => {
  vi.clearAllMocks();
  storeState.mfaRequired = false;
  window.history.pushState({}, "", "/login");
});

afterEach(() => {
  window.history.pushState({}, "", "/");
});

describe("LoginPage", () => {
  it("renders the login AuthFlow (initialPanel=0) and forwards return_to, social error, and invite code", () => {
    window.history.pushState(
      {},
      "",
      "/login?return_to=%2Fdashboard&error=access_denied&code=INVITE-5",
    );

    render(<LoginPage />);

    expect(screen.queryByTestId("mfa-verify-form")).not.toBeInTheDocument();
    const flow = screen.getByTestId("auth-flow");
    expect(JSON.parse(flow.dataset.props ?? "{}")).toEqual({
      initialPanel: 0,
      returnTo: "/dashboard",
      socialError: "access_denied",
      initialInviteCode: "INVITE-5",
    });
  });

  it("omits absent params (only initialPanel) when no query string is present", () => {
    window.history.pushState({}, "", "/login");

    render(<LoginPage />);

    const flow = screen.getByTestId("auth-flow");
    expect(JSON.parse(flow.dataset.props ?? "{}")).toEqual({
      initialPanel: 0,
    });
  });

  it("renders MfaVerifyForm (not AuthFlow) when mfaRequired is true, passing returnTo", () => {
    storeState.mfaRequired = true;
    window.history.pushState({}, "", "/login?return_to=%2Fdashboard");

    render(<LoginPage />);

    expect(screen.queryByTestId("auth-flow")).not.toBeInTheDocument();
    const form = screen.getByTestId("mfa-verify-form");
    expect(JSON.parse(form.dataset.props ?? "{}")).toEqual({
      returnTo: "/dashboard",
    });
  });
});
