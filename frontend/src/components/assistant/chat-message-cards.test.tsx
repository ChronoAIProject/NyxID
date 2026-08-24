import { render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

vi.mock("@/components/assistant/blocks/connect-card", () => ({
  ConnectCard: ({ block }: { block: { reason_code?: string; service_name: string } }) => (
    <div data-testid="connect-card" data-reason={block.reason_code}>
      {block.service_name}
    </div>
  ),
}));

import { ChatMessageBubble } from "@/components/assistant/chat-message";

describe("canonical chat cards", () => {
  it("renders a typed authorization blocker on the assistant message", () => {
    const { container } = render(
      <ChatMessageBubble
        message={{
          id: "assistant-blocked",
          role: "assistant",
          content: "",
          status: "streaming",
          timestamp: 1,
          authorizationBlockers: [
            {
              serviceSlug: "api-github",
              serviceLabel: "GitHub",
              reasonCode: "NYXID_UNAUTHORIZED",
              safeMessage: "Reconnect GitHub to continue.",
            },
          ],
        }}
      />,
    );
    expect(screen.getByTestId("connect-card")).toHaveAttribute(
      "data-reason",
      "NYXID_UNAUTHORIZED",
    );
    expect(screen.getByText("GitHub")).toBeVisible();
    expect(container.querySelector("[data-assistant-halo]")).toBeNull();
    expect(container.querySelector("[data-streaming-dots]")).toBeVisible();
  });
});
