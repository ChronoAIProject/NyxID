import {
  act,
  fireEvent,
  render,
  screen,
  waitFor,
} from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { ApiError } from "@/lib/api-client";
import { MfaStepUpForm } from "./mfa-step-up";
import { useMfaStepUp } from "@/hooks/use-mfa-step-up";
const { request, identity } = vi.hoisted(() => ({
  request: vi.fn(),
  identity: { userId: "first-person" },
}));
vi.mock("@/stores/auth-store", () => ({
  useAuthStore: { getState: () => ({ user: { id: identity.userId } }) },
}));
vi.mock("@/lib/api-client", async (importOriginal) => ({
  ...(await importOriginal<typeof import("@/lib/api-client")>()),
  apiClient: request,
}));
beforeEach(() => {
  vi.clearAllMocks();
  identity.userId = "first-person";
});
const message =
  "Verify with your authenticator to approve sign-ins on other devices";
describe("session step-up", () => {
  it("keeps a wrong-code failure inline and retries only after verification succeeds", async () => {
    const retry = vi.fn().mockResolvedValue(undefined);
    request
      .mockRejectedValueOnce(
        new ApiError(400, {
          error: "bad_request",
          error_code: 1000,
          message: "Invalid MFA code",
        }),
      )
      .mockResolvedValueOnce({});
    render(
      <MfaStepUpForm message={message} onCancel={vi.fn()} onVerified={retry} />,
    );
    const user = userEvent.setup();
    await user.type(screen.getByLabelText("Authenticator code"), "123456");
    await user.click(screen.getByRole("button", { name: "Verify & continue" }));
    expect(await screen.findByRole("alert")).toHaveTextContent(
      "Invalid MFA code",
    );
    expect(retry).not.toHaveBeenCalled();
    await user.click(
      screen.getByRole("button", { name: "Use a recovery code" }),
    );
    await user.type(screen.getByLabelText("Recovery code"), "a".repeat(32));
    await user.click(screen.getByRole("button", { name: "Verify & continue" }));
    await waitFor(() => expect(retry).toHaveBeenCalledTimes(1));
    expect(request).toHaveBeenLastCalledWith(
      "/auth/mfa/step-up",
      expect.objectContaining({
        body: { code: "a".repeat(32) },
        preserveSessionOn401: true,
      }),
    );
  });
  it.each(["request", "account"] as const)("discards a delayed verification after the %s changes", async (changed) => {
    const retry = vi.fn().mockResolvedValue(undefined);
    function Harness({ scope }: { scope: string }) {
      const stepUp = useMfaStepUp(scope);
      return (
        <>
          <button
            onClick={() =>
              stepUp.handle(
                new ApiError(403, {
                  error: "mfa_step_up_required",
                  error_code: 2003,
                  message,
                }),
                retry,
              )
            }
          >
            Approve
          </button>
          {stepUp.prompt}
        </>
      );
    }
    let finish!: () => void;
    request.mockImplementation(
      () =>
        new Promise<void>((resolve) => {
          finish = resolve;
        }),
    );
    const view = render(<Harness scope="person:request-1" />);
    fireEvent.click(screen.getByText("Approve"));
    fireEvent.change(screen.getByLabelText("Authenticator code"), {
      target: { value: "123456" },
    });
    const user = userEvent.setup();
    await user.click(screen.getByRole("button", { name: "Verify & continue" }));
    if (changed === "request") {
      view.rerender(<Harness scope="person:request-2" />);
    } else {
      // The store can change before the parent commits its next render.
      identity.userId = "second-person";
    }
    await act(async () => {
      finish();
    });
    expect(retry).not.toHaveBeenCalled();
    expect(
      screen.queryByRole("region", { name: "Verify this session" }),
    ).not.toBeInTheDocument();
  });
});
