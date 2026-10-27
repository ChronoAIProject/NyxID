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
  ["cancelled", "cancelled"],
  ["delivery_capacity_exhausted", "attachment limit"],
  ["delivery_failed", "delivery failed"],
])("renders %s without an unavailable recovery action", (state, copy) => {
  render(
    <ArtifactDelivery
      delivery={{ ...delivery, state, recoveryAvailable: false }}
    />,
  );
  expect(screen.getByRole("status")).toHaveTextContent(copy);
  expect(screen.queryByRole("button")).not.toBeInTheDocument();
});
