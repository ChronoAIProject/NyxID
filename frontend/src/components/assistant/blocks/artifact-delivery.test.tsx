import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, expect, it, vi } from "vitest";
import { ArtifactDelivery } from "./artifact-delivery";
import { transitionAssistantIdentity } from "@/lib/assistant/identity";
import { assistantHttp } from "@/lib/assistant/assistant-http";
import { nyxAgentTransport } from "@/lib/assistant/nyxagent-transport";
vi.mock("@/lib/assistant/assistant-http", () => ({ assistantHttp: vi.fn() }));
vi.mock("@/lib/assistant/nyxagent-transport", () => ({
  nyxAgentTransport: { history: vi.fn() },
}));
const delivery = {
  state: "delivery_failed",
  count: 0,
  recoveryAvailable: true,
  endpoint: "/reply/artifact-delivery",
  conversationId: "chat",
  messageSeq: 23,
};
beforeEach(() => vi.clearAllMocks());
it("refreshes the original reply page and invalidates history after recovery", async () => {
  vi.mocked(assistantHttp).mockResolvedValue(new Response("{}"));
  const client = new QueryClient();
  const invalidate = vi.spyOn(client, "invalidateQueries");
  render(
    <QueryClientProvider client={client}>
      <ArtifactDelivery delivery={delivery} />
    </QueryClientProvider>,
  );
  fireEvent.click(
    screen.getByRole("button", { name: "Retry attachment delivery" }),
  );
  await waitFor(() => expect(invalidate).toHaveBeenCalled());
  expect(assistantHttp).toHaveBeenCalledWith(delivery.endpoint, {
    method: "POST",
  });
  expect(nyxAgentTransport.history).toHaveBeenCalledWith("chat", 24);
});
it("uses neutral copy and no recovery control for an unknown future outcome", () => {
  render(
    <QueryClientProvider client={new QueryClient()}>
      <ArtifactDelivery delivery={{ ...delivery, state: "future" }} />
    </QueryClientProvider>,
  );
  expect(screen.getByText(/status is unavailable/)).toBeInTheDocument();
  expect(screen.queryByRole("button")).not.toBeInTheDocument();
});

it("ignores recovery completion after the owner changes", async () => {
  let resolve!: (response: Response) => void;
  vi.mocked(assistantHttp).mockReturnValueOnce(
    new Promise((done) => {
      resolve = done;
    }),
  );
  const client = new QueryClient();
  const invalidate = vi.spyOn(client, "invalidateQueries");
  const view = render(
    <QueryClientProvider client={client}>
      <ArtifactDelivery delivery={delivery} />
    </QueryClientProvider>,
  );
  fireEvent.click(
    screen.getByRole("button", { name: "Retry attachment delivery" }),
  );
  transitionAssistantIdentity("another-owner");
  resolve(new Response("{}"));
  await Promise.resolve();
  await Promise.resolve();
  expect(nyxAgentTransport.history).not.toHaveBeenCalled();
  expect(invalidate).not.toHaveBeenCalled();
  view.unmount();
  transitionAssistantIdentity(null);
});
it("ignores recovery completion after the conversation changes", async () => {
  let resolve!: (response: Response) => void;
  vi.mocked(assistantHttp).mockReturnValueOnce(
    new Promise((done) => {
      resolve = done;
    }),
  );
  const client = new QueryClient();
  const view = render(
    <QueryClientProvider client={client}>
      <ArtifactDelivery delivery={delivery} />
    </QueryClientProvider>,
  );
  fireEvent.click(
    screen.getByRole("button", { name: "Retry attachment delivery" }),
  );
  view.rerender(
    <QueryClientProvider client={client}>
      <ArtifactDelivery
        delivery={{
          ...delivery,
          endpoint: "/another-reply",
          conversationId: "another-chat",
        }}
      />
    </QueryClientProvider>,
  );
  resolve(new Response("{}"));
  await Promise.resolve();
  await Promise.resolve();
  expect(nyxAgentTransport.history).not.toHaveBeenCalled();
});
