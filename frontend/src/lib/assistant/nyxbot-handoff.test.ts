import { beforeEach, expect, it, vi } from "vitest";
import { startNyxBotHandoff } from "./nyxbot-handoff";
import { nyxAgentTransport } from "./nyxagent-transport";
import { handoffReceipt, handoffReceiptKey } from "./nyxbot-handoff-receipts";
import { transitionAssistantIdentity } from "./identity";
import { markAdmitted } from "./send-admission";

vi.mock("./nyxagent-transport", () => ({
  nyxAgentTransport: { send: vi.fn() },
}));
const nyxbot = {
  id: "11111111-1111-4111-8111-111111111111",
  kind: "nyxbot" as const,
  name: "NyxBot",
  destroyed: false,
};
const created = `nyxa-${"a".repeat(32)}`;
const proposal = {
  version: 1 as const,
  title: "Create an Agent Key",
  task: "Help me create a scoped Agent Key for my CI.",
};
const key = () => handoffReceiptKey("owner", "source", "message", proposal);
beforeEach(() => {
  localStorage.clear();
  transitionAssistantIdentity(crypto.randomUUID());
  vi.mocked(nyxAgentTransport.send).mockReset();
  vi.mocked(nyxAgentTransport.send).mockImplementation(
    async (_id, _text, onAdopt) => {
      onAdopt(created);
    },
  );
});

it.each([
  ["Create an Agent Key", "Help me create a scoped Agent Key for my CI."],
  [
    "Create a Telegram bot",
    "Create a Telegram bot named Support and link it to my specialist.",
  ],
])(
  "starts a fresh NyxBot thread for %s, with the selected NyxBot and no model override",
  async (title, task) => {
    const adopt = vi.fn();
    await startNyxBotHandoff(
      { version: 1, title, task },
      "source",
      adopt,
      key(),
      nyxbot,
    );
    expect(nyxAgentTransport.send).toHaveBeenCalledWith(
      undefined,
      `${task}\n\n[Specialist chat](/assistant?c=source)`,
      expect.any(Function),
      undefined,
      { agent: nyxbot, clientDraftKey: `handoff:${key()}` },
    );
    expect(adopt).toHaveBeenCalledWith(created);
  },
);

it("reopens the same result after reload/account rebind instead of sending twice", async () => {
  await startNyxBotHandoff(proposal, "source", vi.fn(), key(), nyxbot);
  transitionAssistantIdentity("temporary");
  transitionAssistantIdentity("owner");
  const adopt = vi.fn();
  await startNyxBotHandoff(proposal, "source", adopt, key(), nyxbot);
  expect(adopt).toHaveBeenCalledWith(created);
  expect(nyxAgentTransport.send).toHaveBeenCalledTimes(1);
  expect(localStorage.getItem("nyxid.assistant_handoffs.v1")).not.toContain(
    proposal.task,
  );
});

it("keeps outcomes separate for users, messages and changed proposals", () => {
  const original = key();
  expect(handoffReceiptKey("other", "source", "message", proposal)).not.toBe(
    original,
  );
  expect(handoffReceiptKey("owner", "source", "another", proposal)).not.toBe(
    original,
  );
  expect(
    handoffReceiptKey("owner", "source", "message", {
      ...proposal,
      task: "Another task",
    }),
  ).not.toBe(original);
});

it("retains an admitted-but-unidentified request across reload without replay", async () => {
  vi.mocked(nyxAgentTransport.send).mockRejectedValue(
    markAdmitted(new Error("Connection lost")),
  );
  await expect(
    startNyxBotHandoff(proposal, "source", vi.fn(), key(), nyxbot),
  ).rejects.toThrow();
  transitionAssistantIdentity("reload");
  await expect(
    startNyxBotHandoff(proposal, "source", vi.fn(), key(), nyxbot),
  ).rejects.toMatchObject({ admitted: true });
  expect(handoffReceipt(key())?.state).toBe("accepted");
  expect(nyxAgentTransport.send).toHaveBeenCalledTimes(1);
});

it("permits retry only when the request was rejected before admission", async () => {
  vi.mocked(nyxAgentTransport.send).mockRejectedValueOnce(
    new Error("Capacity"),
  );
  await expect(
    startNyxBotHandoff(proposal, "source", vi.fn(), key(), nyxbot),
  ).rejects.toThrow("Capacity");
  expect(handoffReceipt(key())).toBeUndefined();
  await startNyxBotHandoff(proposal, "source", vi.fn(), key(), nyxbot);
  expect(nyxAgentTransport.send).toHaveBeenCalledTimes(2);
});

it("rejects invalid proposals and unavailable/wrong agents before starting a turn", async () => {
  await expect(
    startNyxBotHandoff(
      { ...proposal, task: "Use nyxid_abcdefghijk" },
      "source",
      vi.fn(),
      key(),
      nyxbot,
    ),
  ).rejects.toThrow();
  await expect(
    startNyxBotHandoff(proposal, "source", vi.fn(), key(), {
      ...nyxbot,
      destroyed: true,
    }),
  ).rejects.toThrow();
  await expect(
    startNyxBotHandoff(proposal, "source", vi.fn(), key(), {
      ...nyxbot,
      kind: "specialist",
    }),
  ).rejects.toThrow();
  expect(nyxAgentTransport.send).not.toHaveBeenCalled();
});
