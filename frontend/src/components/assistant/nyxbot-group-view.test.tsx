import { beforeEach, describe, expect, it, vi } from "vitest";
import { act, fireEvent, render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { useState, type ReactNode } from "react";
import { messageTime } from "@/lib/assistant/message-time";
import type {
  AssistantGroup,
  AssistantGroupMessage,
} from "@/schemas/assistant-nyxagent";

const post = vi.fn();
let group: AssistantGroup;

vi.mock("@/components/assistant/blocks/tool-image", () => ({
  ToolImage: ({ image }: { image: { endpoint: string; label: string } }) => (
    <img src={image.endpoint} alt={image.label} />
  ),
}));

vi.mock("@tanstack/react-router", () => ({ useNavigate: () => vi.fn() }));
vi.mock("@/stores/auth-store", () => ({
  useAuthStore: (select: (state: { user: { id: string } }) => unknown) =>
    select({ user: { id: "owner" } }),
}));
vi.mock("@/hooks/use-nyxbot-agents", () => ({
  useNyxBotAgents: () => ({ data: { agents: [] } }),
}));
vi.mock("@/hooks/use-approvals", () => ({
  useDecideApproval: () => ({ mutateAsync: vi.fn() }),
}));
vi.mock("@/hooks/use-nyxbot-groups", () => ({
  useNyxBotGroupMessages: () => ({
    data: { group, messages: [], pending_actions: [], before_seq: null },
    error: null,
    post: { mutateAsync: post, isPending: false },
    refetch: vi.fn(),
    loadOlder: vi.fn(),
  }),
}));
vi.mock("@/components/assistant/assistant-shell", () => ({
  AssistantShell: ({ children }: { children: ReactNode }) => <>{children}</>,
}));
vi.mock("@/components/assistant/assistant-link-modals", () => ({
  AssistantLinkModalHost: ({ children }: { children: ReactNode }) => (
    <>{children}</>
  ),
}));
vi.mock("@/components/assistant/assistant-engine-sidebar", () => ({
  AssistantEngineSidebar: () => null,
}));
vi.mock("@/components/assistant/nyxbot-agent-details", () => ({
  AgentDetailsSheet: () => null,
}));
vi.mock("@/components/assistant/nyxbot-group-forms", () => ({
  GroupSettingsDialog: () => null,
}));
// The real composer is covered by its own tests; this stand-in keeps its
// contract: content changes are reported, Send hands over the text and the
// recipients slot renders above the input.
vi.mock("@/components/assistant/upload-composer", () => ({
  UploadComposer: (props: {
    recipients?: ReactNode;
    onContentChange?: (content: string) => void;
    onSend: (text: string, uploads?: unknown) => Promise<void>;
  }) => {
    const [text, setText] = useState("");
    return (
      <div>
        {props.recipients}
        <textarea
          aria-label="Message"
          value={text}
          onChange={(event) => {
            setText(event.target.value);
            props.onContentChange?.(event.target.value);
          }}
        />
        <button
          type="button"
          onClick={() =>
            void props.onSend(text, undefined).then(
              () => {
                setText("");
                props.onContentChange?.("");
              },
              () => undefined,
            )
          }
        >
          Send
        </button>
      </div>
    );
  },
}));

import {
  GroupMessageRow,
  NyxAgentGroupPage,
} from "@/components/assistant/nyxbot-group-view";

const NYXBOT = "00000000-0000-4000-8000-000000000001";
const RESEARCHER = "00000000-0000-4000-8000-000000000002";
const WRITER = "00000000-0000-4000-8000-000000000003";
const GONE = "00000000-0000-4000-8000-000000000004";

function makeGroup(defaults: string[]): AssistantGroup {
  return {
    id: "nyxg-00000000000000000000000000000001",
    name: "Launch",
    members: [
      { id: NYXBOT, name: "nyxbot", kind: "nyxbot", destroyed: false, working: false },
      { id: RESEARCHER, name: "researcher", kind: "specialist", destroyed: false, working: false },
      { id: WRITER, name: "writer", kind: "specialist", destroyed: false, working: false },
      { id: GONE, name: "retired", kind: "specialist", destroyed: true, working: false },
    ],
    lead_agent_id: NYXBOT,
    default_recipient_ids: defaults,
    working_agent_ids: [],
    message_count: 0,
    last_message_at: null,
    created_at: "2026-10-01T00:00:00Z",
  } as AssistantGroup;
}

function pills() {
  return within(screen.getByRole("group", { name: "Message recipients" }))
    .queryAllByRole("button", { name: /^Remove recipient / })
    .map((button) => button.textContent?.replace(" ×", ""));
}

async function type(text: string) {
  const box = screen.getByRole("textbox", { name: "Message" });
  await userEvent.clear(box);
  if (text) await userEvent.type(box, text);
}

async function send() {
  await userEvent.click(screen.getByRole("button", { name: "Send" }));
}

describe("group recipient pills", () => {
  beforeEach(() => {
    post.mockReset();
    post.mockResolvedValue({});
    group = makeGroup([NYXBOT]);
  });

  it("seeds the pills from the group's default recipients on load", () => {
    group = makeGroup([RESEARCHER, WRITER]);
    render(<NyxAgentGroupPage groupId={group.id} mock={false} />);
    expect(pills()).toEqual(["researcher", "writer"]);
  });

  it("drops stale and destroyed ids from the defaults", () => {
    group = makeGroup([GONE, "00000000-0000-4000-8000-00000000dead", WRITER]);
    render(<NyxAgentGroupPage groupId={group.id} mock={false} />);
    expect(pills()).toEqual(["writer"]);
  });

  it("shows a pill for each agent typed as a mention, punctuation included", async () => {
    render(<NyxAgentGroupPage groupId={group.id} mock={false} />);
    await type("@researcher, and @writer: please");
    expect(pills()).toEqual(["researcher", "writer"]);
  });

  it("sends typed mentions without explicit ids, like the server routes them", async () => {
    render(<NyxAgentGroupPage groupId={group.id} mock={false} />);
    await type("@researcher go");
    await send();
    expect(post).toHaveBeenCalledWith({
      text: "@researcher go",
      attachmentIds: [],
    });
  });

  it("sends the edited pills explicitly after a removal", async () => {
    group = makeGroup([RESEARCHER, WRITER]);
    render(<NyxAgentGroupPage groupId={group.id} mock={false} />);
    await userEvent.click(
      screen.getByRole("button", { name: "Remove recipient writer" }),
    );
    expect(pills()).toEqual(["researcher"]);
    await type("carry on");
    await send();
    expect(post).toHaveBeenCalledWith({
      text: "carry on",
      attachmentIds: [],
      recipientAgentIds: [RESEARCHER],
    });
  });

  it("brings a removed agent back when it is mentioned again", async () => {
    group = makeGroup([RESEARCHER]);
    render(<NyxAgentGroupPage groupId={group.id} mock={false} />);
    await userEvent.click(
      screen.getByRole("button", { name: "Remove recipient researcher" }),
    );
    expect(pills()).toEqual([]);
    expect(screen.getByRole("status")).toHaveTextContent(
      "Choose at least one recipient.",
    );
    await type("@researcher do X");
    expect(pills()).toEqual(["researcher"]);
    await send();
    expect(post).toHaveBeenCalledWith({
      text: "@researcher do X",
      attachmentIds: [],
    });
  });

  it("keeps a picked recipient while typing and resets after sending", async () => {
    render(<NyxAgentGroupPage groupId={group.id} mock={false} />);
    await userEvent.click(screen.getByRole("button", { name: "Add recipient" }));
    const options = await screen.findAllByRole("menuitem");
    // Destroyed members are never offered.
    expect(options.map((option) => option.textContent)).toEqual([
      "researcher",
      "writer",
    ]);
    await userEvent.click(screen.getByRole("menuitem", { name: "writer" }));
    await type("@researcher hello");
    expect(pills()).toEqual(["researcher", "writer"]);
    await type("plain words");
    expect(pills()).toEqual(["writer"]);
    await send();
    expect(post).toHaveBeenLastCalledWith({
      text: "plain words",
      attachmentIds: [],
      recipientAgentIds: [WRITER],
    });
    expect(pills()).toEqual(["NyxBot"]);
    await type("next");
    await send();
    expect(post).toHaveBeenLastCalledWith({ text: "next", attachmentIds: [] });
  });

  it("refuses to send an emptied selection instead of falling back to the lead", async () => {
    render(<NyxAgentGroupPage groupId={group.id} mock={false} />);
    await userEvent.click(
      screen.getByRole("button", { name: "Remove recipient NyxBot" }),
    );
    await type("anyone?");
    await send();
    expect(post).not.toHaveBeenCalled();
  });
});

describe("delivered message recipients", () => {
  it("renders a member's tool image through the group attachment endpoint", () => {
    group = makeGroup([NYXBOT]);
    const message = {
      id: "reply", group_id: group.id, seq: 2, role: "agent", agent_id: RESEARCHER,
      agent_name: "researcher", text: "Generated image.", created_at: "2026-10-01T00:00:00Z",
      activities: [], recipient_agent_ids: [],
      attachments: [{ id: "image-1", label: "camera__snapshot", origin: "tool", content_type: "image/png", size: 20 }],
    } as unknown as AssistantGroupMessage;
    render(<GroupMessageRow groupId={group.id} message={message} names={[]} members={group.members} onOpenAgent={() => undefined} />);
    expect(screen.getByRole("img", { name: "camera__snapshot" })).toHaveAttribute(
      "src", `/assistant/nyxagent/groups/${group.id}/attachments/image-1`,
    );
  });

  it("names the recipients of a sent message instead of their ids", () => {
    group = makeGroup([NYXBOT]);
    const message = {
      id: "m",
      group_id: group.id,
      seq: 1,
      role: "user",
      text: "@researcher hi",
      created_at: "2026-10-01T00:00:00Z",
      activities: [],
      attachments: [],
      recipient_agent_ids: [RESEARCHER, GONE, "missing"],
    } as unknown as AssistantGroupMessage;
    render(
      <GroupMessageRow
        groupId={group.id}
        message={message}
        names={["researcher"]}
        members={group.members}
        onOpenAgent={() => undefined}
      />,
    );
    expect(
      screen.getByText("to researcher, retired, a removed agent"),
    ).toBeInTheDocument();
    expect(screen.queryByText(new RegExp(RESEARCHER))).toBeNull();
  });
});

describe("group message timestamps", () => {
  it("stamps both the owner's messages and agent replies with their date", () => {
    group = makeGroup([NYXBOT]);
    const base = {
      group_id: group.id,
      activities: [],
      attachments: [],
      created_at: "2026-10-01T00:00:00Z",
    };
    const user = { ...base, id: "u", seq: 1, role: "user", text: "hi" };
    const reply = {
      ...base,
      id: "a",
      seq: 2,
      role: "agent",
      text: "hello",
      created_at: "2026-10-02T09:15:00Z",
      agent: { id: NYXBOT, name: "NyxBot", kind: "nyxbot" },
    };
    const { container } = render(
      <>
        {[user, reply].map((message) => (
          <GroupMessageRow
            key={message.id}
            groupId={group.id}
            message={message as unknown as AssistantGroupMessage}
            names={[]}
            members={group.members}
            onOpenAgent={() => undefined}
          />
        ))}
      </>,
    );
    const times = [...container.querySelectorAll("time")];
    expect(times.map((time) => time.getAttribute("datetime"))).toEqual([
      "2026-10-01T00:00:00.000Z",
      "2026-10-02T09:15:00.000Z",
    ]);
    for (const time of times) {
      const expected = messageTime(time.getAttribute("datetime"))!;
      expect(time).toHaveTextContent(expected.short);
      expect(time).toHaveAttribute("title", expected.label);
      expect(time.getAttribute("title")).toMatch(/2026/);
    }
  });
});


describe("group message body copy integration", () => {
  function row(role: "user" | "agent", text: string) {
    group = makeGroup([NYXBOT]);
    const message = { id: "copy", group_id: group.id, seq: 1, role, text,
      agent_id: NYXBOT, agent_name: "NyxBot", created_at: "2026-10-01T00:00:00Z",
      activities: [], attachments: [], recipient_agent_ids: [] } as unknown as AssistantGroupMessage;
    return <GroupMessageRow groupId={group.id} message={message} names={[]} members={group.members} onOpenAgent={() => undefined} />;
  }
  it("copies the sanitized assistant body with Markdown code intact", async () => {
    const writeText = vi.fn().mockResolvedValue(undefined);
    Object.defineProperty(navigator, "clipboard", { configurable: true, value: { writeText } });
    const markdown = "**Answer**\n\n```ts\n  run()\n```";
    render(row("agent", `${markdown}\n<function_calls>hidden-tool-input</function_calls>`));
    await act(async () => fireEvent.click(screen.getByRole("button", { name: "Copy message" })));
    expect(writeText).toHaveBeenCalledExactlyOnceWith(markdown);
  });
  it("copies original user text without rendered recipients or timestamps", async () => {
    const writeText = vi.fn().mockResolvedValue(undefined);
    Object.defineProperty(navigator, "clipboard", { configurable: true, value: { writeText } });
    const original = "@NyxBot  <function_calls>literal example</function_calls>  ";
    render(row("user", original));
    await act(async () => fireEvent.click(screen.getByRole("button", { name: "Copy message" })));
    expect(writeText).toHaveBeenCalledExactlyOnceWith(original);
  });
  it("has no copy button for output without a visible body", () => {
    render(row("agent", "<function_calls>hidden</function_calls>"));
    expect(screen.queryByRole("button", { name: "Copy message" })).not.toBeInTheDocument();
  });
});
