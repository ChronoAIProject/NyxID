import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { AssistantServiceAccessReviewDialog } from "./assistant-service-access-review-dialog";

const { mockPost } = vi.hoisted(() => ({ mockPost: vi.fn() }));

vi.mock("@/lib/api-client", () => ({
  api: { post: mockPost },
  ApiError: class ApiError extends Error {
    readonly status: number;
    constructor(status: number, message = `HTTP ${String(status)}`) {
      super(message);
      this.status = status;
    }
  },
}));

const SERVICE_ID = "11111111-1111-4111-8111-111111111111";
const PARAMS = {
  userServiceId: SERVICE_ID,
  serviceSlug: "github",
  resourceUri: "https://nyxid.example/api/v1/proxy/s/github",
} as const;

function renderDialog(onComplete = vi.fn(), onOpenChange = vi.fn()) {
  render(
    <AssistantServiceAccessReviewDialog
      open
      onOpenChange={onOpenChange}
      actionRequestId="action-review-1"
      params={PARAMS}
      onComplete={onComplete}
    />,
  );
  return { onComplete, onOpenChange };
}

beforeEach(() => {
  mockPost.mockReset();
});

describe("AssistantServiceAccessReviewDialog", () => {
  it("posts the exact effect and reports only after the user returns", async () => {
    mockPost.mockResolvedValue({
      resource: { userServiceId: SERVICE_ID },
      replayed: false,
    });
    const { onComplete } = renderDialog();

    await userEvent.click(
      screen.getByRole("button", { name: "Approve access" }),
    );

    await waitFor(() => expect(mockPost).toHaveBeenCalledTimes(1));
    expect(mockPost).toHaveBeenCalledWith(
      "/assistant/actions/services/access-review",
      {
        actionRequestId: "action-review-1",
        userServiceId: SERVICE_ID,
        serviceSlug: "github",
        resourceUri: "https://nyxid.example/api/v1/proxy/s/github",
      },
    );
    expect(onComplete).not.toHaveBeenCalled();
    expect(screen.queryByText(PARAMS.resourceUri)).not.toBeInTheDocument();

    await userEvent.click(
      screen.getByRole("button", { name: "Return to chat" }),
    );
    expect(onComplete).toHaveBeenCalledWith(SERVICE_ID);
  });

  it("keeps an error retryable and accepts a lost-response replay", async () => {
    mockPost
      .mockRejectedValueOnce(new Error("Connection interrupted"))
      .mockResolvedValueOnce({
        resource: { userServiceId: SERVICE_ID },
        replayed: true,
      });
    renderDialog();

    await userEvent.click(
      screen.getByRole("button", { name: "Approve access" }),
    );
    expect(await screen.findByRole("alert")).toHaveTextContent(
      "Connection interrupted",
    );
    await userEvent.click(
      screen.getByRole("button", { name: "Approve access" }),
    );

    expect(
      await screen.findByText("Access was already approved"),
    ).toBeInTheDocument();
    expect(mockPost).toHaveBeenCalledTimes(2);
  });

  it("rejects a mismatched effect resource and remains retryable", async () => {
    mockPost
      .mockResolvedValueOnce({
        resource: { userServiceId: "22222222-2222-4222-8222-222222222222" },
        replayed: false,
      })
      .mockResolvedValueOnce({
        resource: { userServiceId: SERVICE_ID },
        replayed: false,
      });
    renderDialog();

    await userEvent.click(
      screen.getByRole("button", { name: "Approve access" }),
    );
    expect(await screen.findByRole("alert")).toHaveTextContent(
      "different service identity",
    );
    await userEvent.click(
      screen.getByRole("button", { name: "Approve access" }),
    );
    expect(await screen.findByText("Access approved")).toBeInTheDocument();
  });

  it("cancels without an effect or report", async () => {
    const { onComplete, onOpenChange } = renderDialog();

    await userEvent.click(screen.getByRole("button", { name: "Cancel" }));

    expect(mockPost).not.toHaveBeenCalled();
    expect(onComplete).not.toHaveBeenCalled();
    expect(onOpenChange).toHaveBeenCalledWith(false);
  });
});
