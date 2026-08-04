import { render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { RegisterPage } from "./register";

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
  window.history.pushState({}, "", "/register");
});

afterEach(() => {
  window.history.pushState({}, "", "/");
});

describe("RegisterPage", () => {
  it("renders the register AuthFlow (initialPanel=1) and forwards parsed return_to + invite code", () => {
    window.history.pushState(
      {},
      "",
      "/register?return_to=%2Fteam&code=INVITE-9",
    );

    render(<RegisterPage />);

    expect(screen.queryByTestId("mfa-verify-form")).not.toBeInTheDocument();
    const flow = screen.getByTestId("auth-flow");
    expect(JSON.parse(flow.dataset.props ?? "{}")).toEqual({
      initialPanel: 1,
      returnTo: "/team",
      initialInviteCode: "INVITE-9",
    });
  });

  it("forwards undefined returnTo/inviteCode when those params are absent", () => {
    window.history.pushState({}, "", "/register");

    render(<RegisterPage />);

    const flow = screen.getByTestId("auth-flow");
    // Missing params serialize as omitted keys (value undefined).
    expect(JSON.parse(flow.dataset.props ?? "{}")).toEqual({
      initialPanel: 1,
    });
  });

  it("renders MfaVerifyForm (not AuthFlow) when mfaRequired is true, passing returnTo", () => {
    storeState.mfaRequired = true;
    window.history.pushState({}, "", "/register?return_to=%2Fteam");

    render(<RegisterPage />);

    expect(screen.queryByTestId("auth-flow")).not.toBeInTheDocument();
    const form = screen.getByTestId("mfa-verify-form");
    expect(JSON.parse(form.dataset.props ?? "{}")).toEqual({
      returnTo: "/team",
    });
  });
});
