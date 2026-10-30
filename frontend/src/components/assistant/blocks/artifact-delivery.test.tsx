import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { ArtifactDelivery } from "./artifact-delivery";
import { assistantHttp } from "@/lib/assistant/assistant-http";
import { ApiError } from "@/lib/api-client";
vi.mock("@/lib/assistant/assistant-http", () => ({ assistantHttp: vi.fn() }));
const delivery = {
  state: "delivery_pending",
  count: 0,
  recoveryAvailable: true,
  endpoint:
    "/assistant/nyxagent/conversations/nyxa-example/messages/message/artifact-delivery",
};
beforeEach(() => vi.clearAllMocks());
afterEach(() => vi.restoreAllMocks());
it("recovers the server-associated message with an empty POST and refreshes history", async () => {
  const reload = vi
    .spyOn(window.location, "reload")
    .mockImplementation(() => {});
  vi.mocked(assistantHttp).mockResolvedValue(new Response());
  render(<ArtifactDelivery delivery={delivery} />);
  const retry = screen.getByRole("button", {
    name: "Retry attachment delivery",
  });
  retry.focus();
  await userEvent.keyboard("{Enter}");
  await waitFor(() => expect(reload).toHaveBeenCalledTimes(1));
  expect(assistantHttp).toHaveBeenCalledExactlyOnceWith(delivery.endpoint, {
    method: "POST",
  });
});
it("shows a distinct expired source and permits another attempt after an HTTP failure", async () => {
  vi.mocked(assistantHttp).mockRejectedValue(
    new ApiError(410, {
      error: "expired",
      error_code: 12101,
      message: "expired",
    }),
  );
  render(<ArtifactDelivery delivery={delivery} />);
  await userEvent.click(
    screen.getByRole("button", { name: "Retry attachment delivery" }),
  );
  await screen.findByText(/generated file has expired/);
  expect(
    screen.getByRole("button", { name: "Retry attachment delivery" }),
  ).toBeEnabled();
});
it.each([
  ["result_expired", "expired before delivery"],
  ["retention_expired", "no longer available under retention policy"],
  ["cancelled", "cancelled"],
  ["delivery_capacity_exhausted", "attachment limit"],
  ["delivery_failed", "delivery failed"],
  ["delivery_missing", "saved generated files are no longer available"],
])("renders %s without an unavailable recovery action", (state, copy) => {
  render(
    <ArtifactDelivery
      delivery={{ ...delivery, state, recoveryAvailable: false }}
    />,
  );
  expect(screen.getByRole("status")).toHaveTextContent(copy);
  expect(screen.queryByRole("button")).not.toBeInTheDocument();
});

it("renders every indexed rejection alongside delivered peers without upstream text", () => {
  render(
    <ArtifactDelivery
      delivery={{
        ...delivery,
        state: "materialized",
        count: 1,
        recoveryAvailable: false,
        rejected: [
          { index: 0, filename: "bad.png", reason: "invalid" },
          { index: 2, filename: "bad.png", reason: "unavailable" },
        ],
      }}
    />,
  );
  expect(screen.getByRole("status")).toHaveTextContent(
    "1 verified file attached",
  );
  expect(
    screen.getByText(/File 1 \(bad.png\): generated but not delivered/),
  ).toBeVisible();
  expect(
    screen.getByText(/File 3 \(bad.png\): generated but not delivered/),
  ).toBeVisible();
  expect(screen.queryByRole("button")).not.toBeInTheDocument();
});
it.each(["source_verification_failed", "artifact_too_large"])(
  "never calls an unverified %s result verified or recoverable",
  (state) => {
    render(
      <ArtifactDelivery
        delivery={{ ...delivery, state, recoveryAvailable: false }}
      />,
    );
    expect(screen.getByRole("status")).toHaveTextContent(
      "No files were attached",
    );
    expect(screen.getByRole("status")).not.toHaveTextContent(
      "result is verified",
    );
    expect(screen.queryByRole("button")).not.toBeInTheDocument();
  },
);
