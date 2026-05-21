import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { ApiError } from "@/lib/api-client";

const { fixtures, mockNavigate, mockRedeem } = vi.hoisted(() => ({
  fixtures: { nonce: "ORGINV-ABC-123" },
  mockNavigate: vi.fn(),
  mockRedeem: vi.fn(),
}));

vi.mock("@tanstack/react-router", () => ({
  useNavigate: () => mockNavigate,
  useParams: () => ({ nonce: fixtures.nonce }),
}));

vi.mock("@/hooks/use-orgs", () => ({
  useRedeemInvite: () => ({
    mutateAsync: mockRedeem,
  }),
}));

import { OrgJoinPage } from "./org-join";

function makeApiError(error: string, message: string): ApiError {
  return new ApiError(410, {
    error,
    error_code: 8105,
    message,
  });
}

describe("OrgJoinPage", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    fixtures.nonce = "ORGINV-ABC-123";
  });

  it("redeems the nonce from the URL on mount", async () => {
    mockRedeem.mockResolvedValue({ org_id: "org-1", role: "member" });

    render(<OrgJoinPage />);

    await waitFor(() => {
      expect(mockRedeem).toHaveBeenCalledWith("ORGINV-ABC-123");
    });
  });

  it("on success shows the redirecting state and navigates to the joined org", async () => {
    mockRedeem.mockResolvedValue({ org_id: "org-77", role: "member" });

    render(<OrgJoinPage />);

    await waitFor(() => {
      expect(screen.getByText("Joined. Redirecting...")).toBeInTheDocument();
    });
    expect(mockNavigate).toHaveBeenCalledWith({
      to: "/orgs/$orgId",
      params: { orgId: "org-77" },
    });
  });

  it("redeems the nonce only once despite StrictMode-style double mount guard", async () => {
    // The page never resolves so we can assert the single-shot ref behaviour
    // without a success/error transition racing the assertion.
    mockRedeem.mockReturnValue(new Promise(() => {}));

    render(<OrgJoinPage />);

    await waitFor(() => {
      expect(mockRedeem).toHaveBeenCalled();
    });
    // Pending UI is shown while the redeem is in flight.
    expect(screen.getByText("Joining organization...")).toBeInTheDocument();
    expect(mockRedeem).toHaveBeenCalledTimes(1);
  });

  it("on an expired invite shows the dedicated expiry message", async () => {
    mockRedeem.mockRejectedValue(
      makeApiError("org_invite_expired", "Invite expired on the wire"),
    );

    render(<OrgJoinPage />);

    await waitFor(() => {
      expect(
        screen.getByText(
          "This invite has expired. Ask an admin to send a new invite.",
        ),
      ).toBeInTheDocument();
    });
    expect(screen.getByText("Could not join organization")).toBeInTheDocument();
    // Expiry is not a success — no navigation away on this branch.
    expect(mockNavigate).not.toHaveBeenCalled();
  });

  it("on a generic ApiError surfaces the server-provided message", async () => {
    mockRedeem.mockRejectedValue(
      makeApiError("org_invite_invalid", "This invite link is not valid."),
    );

    render(<OrgJoinPage />);

    await waitFor(() => {
      expect(
        screen.getByText("This invite link is not valid."),
      ).toBeInTheDocument();
    });
    expect(mockNavigate).not.toHaveBeenCalled();
  });

  it("on a non-ApiError failure shows the generic fallback message", async () => {
    mockRedeem.mockRejectedValue(new Error("network down"));

    render(<OrgJoinPage />);

    await waitFor(() => {
      expect(
        screen.getByText(
          "Failed to redeem invite. The link may be invalid or expired.",
        ),
      ).toBeInTheDocument();
    });
  });

  it("error state offers a Back to organizations action that navigates to /orgs", async () => {
    const user = userEvent.setup();
    mockRedeem.mockRejectedValue(
      makeApiError("org_invite_invalid", "Nope."),
    );

    render(<OrgJoinPage />);

    const backButton = await screen.findByRole("button", {
      name: "Back to organizations",
    });
    await user.click(backButton);

    expect(mockNavigate).toHaveBeenCalledWith({ to: "/orgs" });
  });
});
