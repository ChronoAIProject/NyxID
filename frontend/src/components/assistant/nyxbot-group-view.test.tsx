import { beforeEach, describe, expect, it, vi } from "vitest";
import { act, fireEvent, render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { useState, type ReactNode } from "react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { messageTime } from "@/lib/assistant/message-time";
import type {
  AssistantGroup,
  AssistantGroupMessage,
} from "@/schemas/assistant-nyxagent";

vi.mock("@/hooks/use-collaboration-groups", async (importOriginal) => ({
  ...await importOriginal<typeof import("@/hooks/use-collaboration-groups")>(),
  useCollaborationRead: vi.fn(),
}));

vi.mock("./blocks/crm-artifact-block", () => ({
  CrmArtifactBlock: ({ artifacts }: { artifacts?: readonly { title: string }[] }) =>
    artifacts?.length ? <div aria-label="CRM artifact">{artifacts[0]?.title}</div> : null,
}));

const post = vi.fn();
let group: AssistantGroup;
let groupLoaded = true;
let groupError: Error | null = null;
let shownMentions: readonly { id: string; name: string; shared?: boolean }[] = [];
vi.mock("./shared-agent-controls", () => ({
  SharedConsentCards: ({ agentId, conversationId }: { agentId: string; conversationId: string }) => <div data-testid="shared-consents">{agentId}:{conversationId}</div>,
}));

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
    data: groupLoaded ? { group, messages: [], pending_actions: [], before_seq: null } : undefined,
    error: groupError,
    post: { mutateAsync: post, isPending: false },
    refetch: vi.fn(),
    loadOlder: vi.fn(),
  }),
}));
vi.mock("@/components/assistant/assistant-shell", () => ({
  AssistantShell: ({ children, contentTitle }: { children: ReactNode; contentTitle?: boolean }) => <main data-content-title={Boolean(contentTitle)}>{children}</main>,
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
    disabled?: boolean;
    placeholder?: string;
    mentions?: readonly { id: string; name: string; shared?: boolean }[];
    recipients?: ReactNode;
    onContentChange?: (content: string) => void;
    onSend: (text: string, uploads?: unknown) => Promise<void>;
  }) => {
    shownMentions = props.mentions ?? [];
    const [text, setText] = useState("");
    return (
      <div>
        {props.recipients}
        <textarea
          aria-label="Message"
          placeholder={props.placeholder}
          disabled={props.disabled}
          value={text}
          onChange={(event) => {
            setText(event.target.value);
            props.onContentChange?.(event.target.value);
          }}
        />
        <button
          type="button"
          disabled={props.disabled}
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
    groupLoaded = true;
    groupError = null;
    post.mockReset();
    post.mockResolvedValue({});
    group = makeGroup([NYXBOT]);
  });

  it("replaces the loading group header with recovery when loading fails", () => {
    groupLoaded = false;
    groupError = new Error("Group is unavailable");
    render(<NyxAgentGroupPage groupId={group.id} mock={false} />);
    expect(screen.getByRole("alert")).toHaveTextContent("Could not open this group");
    expect(screen.getByRole("button", { name: "Back to home" })).toBeVisible();
    expect(screen.queryByText("Loading group…")).not.toBeInTheDocument();
    expect(screen.queryByText("Loading members…")).not.toBeInTheDocument();
  });

  it("keeps one group header while its identity loads", () => {
    groupLoaded = false;
    const view = render(<NyxAgentGroupPage groupId={group.id} mock={false} />);
    expect(screen.getByRole("main")).toHaveAttribute("data-content-title", "true");
    const panels = view.container.querySelector("[data-chat-thread-panels]");
    expect(panels).not.toBeNull();
    expect(screen.getByRole("heading", { name: "Loading group…" })).toBeVisible();
    groupLoaded = true;
    view.rerender(<NyxAgentGroupPage groupId={group.id} mock={false} />);
    expect(view.container.querySelectorAll("[data-chat-thread-panels]")).toHaveLength(1);
    expect(view.container.querySelector("[data-chat-thread-panels]")).toBe(panels);
    expect(screen.getByRole("heading", { name: "Launch" })).toBeVisible();
    expect(screen.getByRole("main")).toHaveAttribute("data-content-title", "true");
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

  it("binds typed mentions to exact recipient ids", async () => {
    render(<NyxAgentGroupPage groupId={group.id} mock={false} />);
    await type("@researcher go");
    await send();
    expect(post).toHaveBeenCalledWith({
      text: "@researcher go",
      attachmentIds: [],
      recipientAgentIds: [RESEARCHER],
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
      recipientAgentIds: [RESEARCHER],
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
    expect(post).toHaveBeenLastCalledWith({ text: "next", attachmentIds: [], recipientAgentIds: [NYXBOT] });
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
  it("attributes an unavailable-agent refusal to its display name", () => {
    const message: AssistantGroupMessage = { id: "notice", seq: 1, role: "notice", text: "This agent cannot reply with your current access.", agent: { id: RESEARCHER, name: "researcher", display_name: "Office Agent", kind: "specialist" }, created_at: "2026-10-01T00:00:00Z" };
    render(<GroupMessageRow groupId="group" message={message} names={[]} onOpenAgent={() => undefined} />);
    expect(screen.getByRole("note")).toHaveTextContent("Office Agent: This agent cannot reply with your current access.");
  });

  it("renders CRM artifact-only agent replies in the group transcript", () => {
    const message: AssistantGroupMessage = {
      id: "crm-reply", seq: 2, role: "agent", agent: null, text: "",
      created_at: "2026-11-06T00:00:00Z",
      crm_artifacts: [{ kind: "object", id: "dataset", owner_id: "owner", object_key: "research", title: "Research dataset", role: "created" }],
    };
    render(<GroupMessageRow groupId="group" message={message} names={[]} onOpenAgent={() => undefined} />);
    expect(within(screen.getByRole("article")).getByLabelText("CRM artifact")).toHaveTextContent("Research dataset");
  });

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
    expect(screen.getByText("to researcher, retired, a removed agent").parentElement).toHaveClass("bg-chat-bubble", "text-chat-bubble-foreground");
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
  it("copies the readable assistant body with code content intact", async () => {
    const writeText = vi.fn().mockResolvedValue(undefined);
    Object.defineProperty(navigator, "clipboard", { configurable: true, value: { writeText } });
    const markdown = "**Answer**\n\n```ts\n  run()\n```";
    render(row("agent", `${markdown}\n<function_calls>hidden-tool-input</function_calls>`));
    await act(async () => fireEvent.click(screen.getByRole("button", { name: "Copy message" })));
    expect(writeText).toHaveBeenCalledExactlyOnceWith("Answer\n\n  run()\n");
  });
  it("copies original user text without rendered recipients or timestamps", async () => {
    const writeText = vi.fn().mockResolvedValue(undefined);
    Object.defineProperty(navigator, "clipboard", { configurable: true, value: { writeText } });
    const original = "@NyxBot  **literal** <function_calls>literal example</function_calls>  ";
    render(row("user", original));
    await act(async () => fireEvent.click(screen.getByRole("button", { name: "Copy message" })));
    expect(writeText).toHaveBeenCalledExactlyOnceWith(original);
  });
  it("has no copy button for output without a visible body", () => {
    render(row("agent", "<function_calls>hidden</function_calls>"));
    expect(screen.queryByRole("button", { name: "Copy message" })).not.toBeInTheDocument();
  });
  it.each(["**Alice**", "[Alice](https://example.test)"])("copies a group's displayed mention name literally: %s", async (label) => {
    const writeText = vi.fn().mockResolvedValue(undefined);
    Object.defineProperty(navigator, "clipboard", { configurable: true, value: { writeText } });
    const member = { id: RESEARCHER, name: "researcher", display_name: label, kind: "specialist" as const, destroyed: false, working: false, mention_handle: "agent-0123456789abcdef" };
    const message: AssistantGroupMessage = { id: "copy-mention", seq: 1, role: "agent", agent: member, text: `Hello @${RESEARCHER} and **team**.`, created_at: "2026-11-03T00:00:00Z" };
    const view = render(<GroupMessageRow groupId={group.id} message={message} names={[]} members={[member]} onOpenAgent={() => undefined} />);
    expect(screen.getByText(`@${label}`)).toHaveAttribute("data-group-mention");
    await act(async () => fireEvent.click(screen.getByRole("button", { name: "Copy message" })));
    expect(writeText).toHaveBeenCalledExactlyOnceWith(`Hello @${label} and team.`);
    writeText.mockClear();
    view.rerender(<GroupMessageRow groupId={group.id} message={{ ...message, role: "user" }} names={[]} members={[member]} onOpenAgent={() => undefined} />);
    await act(async () => fireEvent.click(screen.getByRole("button", { name: "Copy message" })));
    expect(writeText).toHaveBeenCalledExactlyOnceWith(`Hello @${label} and **team**.`);
  });
});

describe("shared crew members", () => {
  beforeEach(() => {
    groupLoaded = true; groupError = null; post.mockReset(); post.mockResolvedValue({});
    group = makeGroup([RESEARCHER]);
    group.members[1] = { ...group.members[1]!, shared: true, owner_name: "Publisher", conversation_id: "shared-thread" };
  });
  it("shows shared picker entries, pills and consent cards for the member thread", () => {
    render(<NyxAgentGroupPage groupId={group.id} mock={false} />);
    expect(shownMentions.find((m) => m.id === RESEARCHER)).toMatchObject({ shared: true, name: "researcher" });
    expect(pills()).toEqual(["researcher · Shared agent · Publisher"]);
    expect(screen.getByTestId("shared-consents")).toHaveTextContent(`${RESEARCHER}:shared-thread`);
  });
  it("binds a plain-text send to the displayed sticky shared recipient", async () => {
    render(<NyxAgentGroupPage groupId={group.id} mock={false} />);
    await type("carry on");
    await send();
    expect(post).toHaveBeenCalledWith({ text: "carry on", attachmentIds: [], recipientAgentIds: [RESEARCHER] });
  });
  it("keeps unavailable members visible but removes mentions, recipients and consent cards", () => {
    group.members[1] = { ...group.members[1]!, unavailable: true };
    render(<NyxAgentGroupPage groupId={group.id} mock={false} />);
    expect(screen.getByRole("button", { name: /researcher \(unavailable\)/ })).toBeVisible();
    expect(shownMentions.some((m) => m.id === RESEARCHER)).toBe(false);
    expect(pills()).toEqual([]);
    expect(screen.queryByTestId("shared-consents")).not.toBeInTheDocument();
  });
  it("inserts exact member IDs for colliding names and routes that UUID", async () => {
    group.members[2] = { ...group.members[2]!, name: "researcher" };
    render(<NyxAgentGroupPage groupId={group.id} mock={false} />);
    expect(shownMentions.find((m) => m.id === RESEARCHER)?.name).toBe(RESEARCHER);
    await type(`@${RESEARCHER} hello`);
    expect(pills()).toEqual(["researcher · Shared agent · Publisher"]);
    await send();
    expect(post).toHaveBeenCalledWith(expect.objectContaining({ text: `@${RESEARCHER} hello`, recipientAgentIds: [RESEARCHER] }));
  });
  it("sends an unavailable UUID to the server for refusal instead of addressing the lead", async () => {
    group.default_recipient_ids = [NYXBOT];
    group.members[1] = { ...group.members[1]!, unavailable: true };
    render(<NyxAgentGroupPage groupId={group.id} mock={false} />);
    await type(`@${RESEARCHER} hello`);
    await send();
    expect(post).toHaveBeenCalledWith(expect.objectContaining({ recipientAgentIds: [RESEARCHER] }));
  });
  it("preserves a destroyed member's UUID and displays the specific refusal", async () => {
    group.default_recipient_ids = [NYXBOT];
    post.mockRejectedValue(new Error("retired was destroyed and can't be messaged"));
    render(<NyxAgentGroupPage groupId={group.id} mock={false} />);
    expect(shownMentions.some((member) => member.id === GONE)).toBe(false);
    await type(`@${GONE} hello`);
    await send();
    expect(post).toHaveBeenCalledWith(expect.objectContaining({ recipientAgentIds: [GONE] }));
    expect(await screen.findByText("retired was destroyed and can't be messaged")).toBeVisible();
  });
  it("leaves ambiguous bare handles for server validation even alongside an exact UUID", async () => {
    group.members[2] = { ...group.members[2]!, name: "researcher" };
    render(<NyxAgentGroupPage groupId={group.id} mock={false} />);
    await type(`@${NYXBOT} and @researcher hello`);
    await send();
    expect(post).toHaveBeenCalledWith({ text: `@${NYXBOT} and @researcher hello`, attachmentIds: [] });
  });
});

it("attributes collaboration guest replies to the requester and owner-paid lane", () => {
  const message: AssistantGroupMessage = { id: "reply", seq: 2, role: "agent", agent: { id: RESEARCHER, name: "Researcher", kind: "specialist" }, text: "Hello Bob", created_at: "2026-10-31T00:00:00Z", requester_user_id: "bob", execution_mode: "guest" };
  render(<GroupMessageRow groupId="collaboration" message={message} names={[]} participants={[{ id: "bob", display_name: "Bob" }]} onOpenAgent={() => undefined} />);
  expect(screen.getByText("For Bob · Guest reply (owner pays)")).toBeVisible();
});

describe("collaboration participant composer", () => {
  beforeEach(() => {
    groupLoaded = true;
    groupError = null;
    post.mockReset().mockResolvedValue({});
    group = makeGroup([]);
    group.members = [{ ...group.members[1]!, display_name: "Office Agent", unavailable: true, unavailable_reason: "not_shared_with_you", owner_name: "Share Ops" }];
    group.collaboration = { version: 1, creator_user_id: "someone-else", membership_revision: 1, history_disclosure: "Public history", can_manage: false, your_request_ids: [], transfer_to_user_id: null, guest_access: [{ agent_id: RESEARCHER, enabled: false, revision: 1, can_enable: false, can_disable: false, can_request: true, lane: null }], proposals: [], invites: [] };
  });
  it("lists people and agents and suppresses sticky agents for a people-only draft", async () => {
    group.members = [{ ...group.members[0]!, unavailable: false }];
    group.default_recipient_ids = [RESEARCHER];
    group.participants = [{ id: "shining", display_name: "Shining", handle: "person-0123456789abcdef" }];
    render(<QueryClientProvider client={new QueryClient()}><NyxAgentGroupPage groupId={group.id} mock={false} /></QueryClientProvider>);
    expect(screen.getByPlaceholderText("Message the group — @mention a person or agent")).toBeVisible();
    expect(shownMentions.map((m) => m.id)).toEqual([RESEARCHER, "shining"]);
    await type("@person-0123456789abcdef hello");
    expect(pills()).toEqual([]);
    await send();
    expect(post).toHaveBeenLastCalledWith({ text: "@person-0123456789abcdef hello", attachmentIds: [], recipientAgentIds: [] });
    expect(pills()).toEqual(["Office Agent"]);
    await type("@person-0123456789abcdef @researcher help");
    await send();
    expect(post).toHaveBeenLastCalledWith({ text: "@person-0123456789abcdef @researcher help", attachmentIds: [], recipientAgentIds: [RESEARCHER] });
  });
  it("renders current and unknown UUID mentions as chips in every public message role", () => {
    const member = { id: RESEARCHER, name: "office", display_name: "Office Agent", kind: "specialist" as const, destroyed: false, working: false, mention_handle: "agent-0123456789abcdef" };
    const unknown = "00000000-0000-4000-8000-000000000099";
    const message: AssistantGroupMessage = { id: "ids", seq: 1, role: "agent", agent: member, text: `Ask @${RESEARCHER} and @${unknown}.`, created_at: "2026-11-03T00:00:00Z" };
    const view = render(<GroupMessageRow groupId={group.id} message={message} names={[]} members={[member]} onOpenAgent={() => undefined} />);
    for (const role of ["agent", "user", "notice"] as const) {
      view.rerender(<GroupMessageRow groupId={group.id} message={{ ...message, role }} names={[]} members={[member]} onOpenAgent={() => undefined} />);
      expect(screen.getByText("@Office Agent")).toHaveAttribute("data-group-mention");
      expect(screen.getByText("@unknown member")).toHaveAttribute("data-group-mention");
      expect(view.container).not.toHaveTextContent(RESEARCHER);
      expect(view.container).not.toHaveTextContent(unknown);
    }
  });
  it("renders resolved people chips in human and agent messages", () => {
    const participants = [{ id: "shining", display_name: "Shining", handle: "person-0123456789abcdef" }];
    const message: AssistantGroupMessage = { id: "mention", seq: 1, role: "user", agent: null, text: "@person-0123456789abcdef hello", mentioned_participant_ids: ["shining"], created_at: "2026-11-03T00:00:00Z" };
    const view = render(<GroupMessageRow groupId={group.id} message={message} names={[]} participants={participants} onOpenAgent={() => undefined} />);
    expect(screen.queryByLabelText("Mentioned people")).not.toBeInTheDocument();
    expect(screen.getAllByText("@Shining")).toHaveLength(1);
    expect(screen.getByText("@Shining")).toHaveAttribute("data-group-mention");
    view.rerender(<GroupMessageRow groupId={group.id} message={{ ...message, role: "agent", agent: { id: RESEARCHER, name: "researcher", kind: "specialist" } }} names={[]} participants={participants} onOpenAgent={() => undefined} />);
    expect(screen.queryByLabelText("Mentioned people")).not.toBeInTheDocument();
    expect(screen.getAllByText("@Shining")).toHaveLength(1);
    expect(screen.getByText("@Shining")).toHaveAttribute("data-group-mention");
    // Hidden protocol markup may leave resolved metadata behind. Do not
    // resurrect it as a separate visible mention row.
    view.rerender(<GroupMessageRow groupId={group.id} message={{ ...message, role: "agent", text: "<function_calls>@person-0123456789abcdef</function_calls>Done", agent: { id: RESEARCHER, name: "researcher", kind: "specialist" } }} names={[]} participants={participants} onOpenAgent={() => undefined} />);
    expect(screen.queryByText("@Shining")).not.toBeInTheDocument();
    expect(screen.getByText("Done")).toBeVisible();
  });
  it("enables people-only chat without any usable agent and retains server display names", async () => {
    render(<QueryClientProvider client={new QueryClient()}><NyxAgentGroupPage groupId={group.id} mock={false} /></QueryClientProvider>);
    expect(screen.getByRole("textbox", { name: "Message" })).toBeEnabled();
    expect(screen.getByText("Everyone in the group · no agent reply")).toBeVisible();
    expect(screen.getByRole("button", { name: /Office Agent/ })).toBeInTheDocument();
    await type("Hello Shining");
    await send();
    expect(post).toHaveBeenCalledWith({ text: "Hello Shining", attachmentIds: [], recipientAgentIds: [] });
  });
  it("shows unavailable reasons and ask-owner in the recipient picker", async () => {
    const client = new QueryClient();
    render(<QueryClientProvider client={client}><NyxAgentGroupPage groupId={group.id} mock={false} /></QueryClientProvider>);
    await userEvent.click(screen.getByRole("button", { name: "Add recipient" }));
    expect(screen.getByText("Not shared with you — ask Share Ops to allow participants.")).toBeVisible();
    expect(screen.getByRole("button", { name: "Ask Share Ops to allow participants" })).toBeVisible();
    client.clear();
  });
  it("allows deliberately deselecting an available agent for a people-only post", async () => {
    group.members = [{ ...group.members[0]!, unavailable: false }];
    group.default_recipient_ids = [RESEARCHER];
    render(<QueryClientProvider client={new QueryClient()}><NyxAgentGroupPage groupId={group.id} mock={false} /></QueryClientProvider>);
    await userEvent.click(screen.getByRole("button", { name: "Remove recipient Office Agent" }));
    await type("Just talking to people");
    await send();
    expect(post).toHaveBeenCalledWith({ text: "Just talking to people", attachmentIds: [], recipientAgentIds: [] });
  });
});
