import { act, cleanup, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { ApiError } from "@/lib/api-client";
import { IdentityResyncSection } from "./identity-resync-section";

const { mutate, successToast, errorToast } = vi.hoisted(() => ({
  mutate: vi.fn(),
  successToast: vi.fn(),
  errorToast: vi.fn(),
}));
vi.mock("@/hooks/use-services", () => ({
  useResyncServiceIdentity: () => ({ mutate, isPending: false }),
}));
vi.mock("sonner", () => ({
  toast: { success: successToast, error: errorToast },
}));

const service = { id: "svc-1", name: "CMA", can_resync_identity: true };
const trigger = () =>
  screen.getByRole("button", { name: "Resync connection identity" });

afterEach(cleanup);
beforeEach(() => vi.clearAllMocks());

describe("IdentityResyncSection", () => {
  it.each([false, undefined])(
    "renders nothing when the server flag is %s",
    (flag) => {
      const { container } = render(
        <IdentityResyncSection
          service={{ ...service, can_resync_identity: flag }}
        />,
      );
      expect(container).toBeEmptyDOMElement();
    },
  );

  it("confirms before overwriting every connection, customized ones included", async () => {
    const user = userEvent.setup();
    render(<IdentityResyncSection service={service} />);

    await user.click(trigger());
    const dialog = screen.getByRole("dialog", {
      name: "Resync connection identity?",
    });
    expect(dialog).toHaveTextContent(
      "overwrites the identity-forwarding settings on every connection to CMA, including connections whose settings were customized",
    );
    expect(dialog).not.toHaveTextContent(/403|fix/i);
    expect(mutate).not.toHaveBeenCalled();

    await user.click(screen.getByRole("button", { name: "Cancel" }));
    expect(mutate).not.toHaveBeenCalled();
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
  });

  it("shows how many connections were checked and updated", async () => {
    const user = userEvent.setup();
    render(<IdentityResyncSection service={service} />);

    await user.click(trigger());
    await user.click(screen.getByRole("button", { name: "Resync" }));
    expect(mutate).toHaveBeenCalledWith("svc-1", expect.any(Object));
    act(() =>
      mutate.mock.calls[0]?.[1].onSuccess({
        catalog_service_id: "svc-1",
        matched_count: 37,
        changed_count: 0,
      }),
    );

    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
    expect(screen.getByText("37 checked, 0 updated")).toBeInTheDocument();
    expect(successToast).toHaveBeenCalledWith(
      "Resync finished: 37 checked, 0 updated",
    );
  });

  it("keeps the dialog open and reports a refusal", async () => {
    const user = userEvent.setup();
    render(<IdentityResyncSection service={service} />);

    await user.click(trigger());
    await user.click(screen.getByRole("button", { name: "Resync" }));
    act(() =>
      mutate.mock.calls[0]?.[1].onError(
        new ApiError(403, {
          error: "forbidden",
          error_code: 1002,
          message: "Only an Admin or the service creator can resync identity",
        }),
      ),
    );

    expect(errorToast).toHaveBeenCalledWith(
      "Only an Admin or the service creator can resync identity",
    );
    expect(screen.getByRole("dialog")).toBeInTheDocument();
  });
});
