import {
  act,
  cleanup,
  fireEvent,
  render,
  screen,
  waitFor,
} from "@testing-library/react";
import {
  QueryClient,
  QueryClientProvider,
  useQuery,
} from "@tanstack/react-query";
import { useLayoutEffect, useState } from "react";
import { afterEach, expect, it, vi } from "vitest";
import { AccountBoundary } from "./account-boundary";
import { ChatMessageList } from "./assistant/chat-message";
import { useAuthStore } from "@/stores/auth-store";
import { bindAccountQueryClient } from "@/lib/account-query-cache";
import { assistantJson } from "@/lib/assistant/assistant-http";
import type { User } from "@/types/api";
import type { ChatMessage } from "@/lib/assistant/chat-types";

const person = (id: string) => ({ id, email: `${id}@example.test` }) as User;
afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
});
it("renders the real Assistant transcript without old text at the first B/A commit, dropping local drafts and delayed responses", async () => {
  useAuthStore.getState().setUser(person("A"));
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  });
  const stop = bindAccountQueryClient(client);
  let finish!: (response: Response) => void;
  let delayed = false;
  const requests = vi.fn(async () => {
    const owner = useAuthStore.getState().user!.id;
    if (delayed)
      return new Promise<Response>((done) => {
        finish = done;
      });
    return Response.json({
      text: `${owner} recognisable private conversation`,
    });
  });
  vi.stubGlobal("fetch", requests);
  globalThis.__nyxidAssistantHttpMock = undefined;
  const commits: { owner: string; text: string }[] = [];
  function Conversation() {
    const owner = useAuthStore((state) => state.user!.id);
    const [draft, setDraft] = useState("");
    const history = useQuery({
      queryKey: ["conversation"],
      queryFn: () => assistantJson<{ text: string }>("/private-conversation"),
    });
    useLayoutEffect(() => {
      commits.push({ owner, text: document.body.textContent ?? "" });
    });
    const messages: ChatMessage[] = history.data
      ? [
          {
            id: owner,
            role: "assistant",
            content: history.data.text,
            timestamp: 1,
            status: "complete",
          },
        ]
      : [];
    return (
      <>
        <span>{owner} identity</span>
        <input
          aria-label="Draft"
          value={draft}
          onChange={(event) => setDraft(event.target.value)}
        />
        <ChatMessageList
          session={{
            clientId: "one",
            conversationId: "one",
            messages,
            expectedTurnCount: 0,
            status: "completed_text",
            title: "one",
          }}
          bottomInset={0}
        />
      </>
    );
  }
  render(
    <QueryClientProvider client={client}>
      <AccountBoundary>
        <Conversation />
      </AccountBoundary>
    </QueryClientProvider>,
  );
  await screen.findByText("A recognisable private conversation");
  fireEvent.change(screen.getByRole("textbox"), {
    target: { value: "A draft" },
  });
  expect(screen.getByRole("textbox")).toHaveValue("A draft");
  act(() =>
    useAuthStore
      .getState()
      .setUser({ ...person("A"), display_name: "same user" }),
  );
  expect(screen.getByRole("textbox")).toHaveValue("A draft");
  delayed = true;
  const refresh = client.refetchQueries();
  await waitFor(() => expect(finish).toBeDefined());
  const releaseA = finish;
  delayed = false;
  act(() => useAuthStore.getState().setUser(person("B")));
  expect(screen.queryByText("A recognisable private conversation")).toBeNull();
  await act(async () =>
    releaseA(Response.json({ text: "delayed A private response" })),
  );
  await refresh;
  await act(async () => {
    await client.refetchQueries();
  });
  await screen.findByText("B recognisable private conversation");
  expect(screen.getByRole("textbox")).toHaveValue("");
  for (const commit of commits.filter((row) => row.owner === "B")) {
    expect(commit.text).not.toContain("A recognisable");
    expect(commit.text).not.toContain("delayed A");
  }
  act(() => useAuthStore.getState().setUser(person("A")));
  expect(screen.queryByText("B recognisable private conversation")).toBeNull();
  await screen.findByText("A recognisable private conversation");
  const last = commits.filter((row) => row.owner === "A").at(-1)!;
  expect(last.text).not.toContain("B recognisable");
  stop();
  client.clear();
});
