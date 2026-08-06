import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

const mocks = vi.hoisted(() => ({
  configure: vi.fn(),
  disable: vi.fn(),
  rotate: vi.fn(),
  toastError: vi.fn(),
  toastSuccess: vi.fn(),
}));

vi.mock("@/hooks/use-connection-webhooks", () => ({
  useConfigureConnectionWebhook: () => ({
    mutateAsync: mocks.configure,
    isPending: false,
  }),
  useDisableConnectionWebhook: () => ({
    mutateAsync: mocks.disable,
    isPending: false,
  }),
  useRotateConnectionWebhookSecret: () => ({
    mutateAsync: mocks.rotate,
    isPending: false,
  }),
}));

vi.mock("sonner", () => ({
  toast: { error: mocks.toastError, success: mocks.toastSuccess },
}));

import { ConnectionWebhookSection } from "./connection-webhook-section";

beforeEach(() => {
  vi.clearAllMocks();
  mocks.configure.mockResolvedValue({
    client_id: "client-1",
    connection_webhook_url: "https://events.example.com/nyxid",
    connection_webhook_enabled: true,
    signing_secret: "nyx_whsec_once",
  });
  mocks.rotate.mockResolvedValue({
    client_id: "client-1",
    connection_webhook_url: "https://events.example.com/nyxid",
    connection_webhook_enabled: true,
    signing_secret: "nyx_whsec_rotated",
  });
  mocks.disable.mockResolvedValue({});
});

describe("ConnectionWebhookSection", () => {
  it("dirty-gates HTTPS configuration and reveals the returned secret once", async () => {
    const user = userEvent.setup();
    render(
      <ConnectionWebhookSection
        clientId="client-1"
        webhookUrl={null}
        enabled={false}
      />,
    );

    const submit = screen.getByRole("button", { name: "Configure Webhook" });
    expect(submit).toBeDisabled();

    await user.type(
      screen.getByLabelText("Webhook URL"),
      "https://events.example.com/nyxid",
    );
    expect(submit).toBeEnabled();
    await user.click(submit);

    await waitFor(() =>
      expect(mocks.configure).toHaveBeenCalledWith({
        clientId: "client-1",
        url: "https://events.example.com/nyxid",
      }),
    );
    expect(screen.getByText("Save Connection Webhook Secret")).toBeInTheDocument();
    expect(screen.getByText("nyx_whsec_once")).toBeInTheDocument();
    expect(screen.getByText(/shown only once/i)).toBeInTheDocument();
  });

  it("confirms rotation and disabling for a configured webhook", async () => {
    const user = userEvent.setup();
    render(
      <ConnectionWebhookSection
        clientId="client-1"
        webhookUrl="https://events.example.com/nyxid"
        enabled
      />,
    );

    expect(screen.getByText("Enabled")).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Rotate Secret" }));
    await user.click(
      screen.getByRole("button", { name: "Confirm Rotation" }),
    );
    await waitFor(() => expect(mocks.rotate).toHaveBeenCalledWith("client-1"));
    expect(screen.getByText("nyx_whsec_rotated")).toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: "I have saved it" }));
    await user.click(screen.getByRole("button", { name: "Disable Webhook" }));
    await user.click(screen.getByRole("button", { name: "Confirm Disable" }));
    await waitFor(() => expect(mocks.disable).toHaveBeenCalledWith("client-1"));
  });
});
