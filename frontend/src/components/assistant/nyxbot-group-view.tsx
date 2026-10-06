import { MachineToolCard } from "./machine-tool-card";
import {
  Fragment,
  useLayoutEffect,
  useRef,
  useState,
  type UIEvent,
} from "react";
import { useNavigate } from "@tanstack/react-router";
import { Settings2 } from "lucide-react";
import { toast } from "sonner";
import { AssistantShell } from "@/components/assistant/assistant-shell";
import { AssistantLinkModalHost } from "@/components/assistant/assistant-link-modals";
import { AssistantEngineSidebar } from "@/components/assistant/assistant-engine-sidebar";
import { UploadComposer } from "@/components/assistant/upload-composer";
import { ToolImage } from "@/components/assistant/blocks/tool-image";
import { TextBlock } from "@/components/assistant/blocks/text-block";
import { AgentAvatar } from "@/components/assistant/nyxbot-agent-avatar";
import { AgentDetailsSheet } from "@/components/assistant/nyxbot-agent-details";
import { GroupSettingsDialog } from "@/components/assistant/nyxbot-group-forms";
import { NyxBotSettingsButton } from "@/components/assistant/nyxbot-settings-dialog";
import { Button } from "@/components/ui/button";
import { useDecideApproval } from "@/hooks/use-approvals";
import { useNyxBotAgents } from "@/hooks/use-nyxbot-agents";
import { useNyxBotGroupMessages } from "@/hooks/use-nyxbot-groups";
import { sanitizeAssistantMessageContent } from "@/lib/assistant/chat-content";
import { nyxAgentTransport } from "@/lib/assistant/nyxagent-transport";
import {
  agentHandle,
  agentTitle,
  groupWithDisplayNames,
  withDisplayName,
  workingLabel,
} from "@/lib/assistant/nyxbot-labels";
import { mentionSegments } from "@/lib/assistant/nyxbot-mentions";
import { cn, formatClockTime } from "@/lib/utils";
import type {
  AssistantGroup,
  AssistantGroupMember,
  AssistantGroupMessage,
  AssistantGroupPendingAction,
} from "@/schemas/assistant-nyxagent";
import { useAuthStore } from "@/stores/auth-store";

export const GROUP_COMPOSER_PLACEHOLDER = "Message the group — @mention an agent";

function WorkingDots() {
  return (
    <span aria-hidden="true" className="flex items-center gap-1">
      {[0, 120, 240].map((delay) => (
        <span
          key={delay}
          className="h-1.5 w-1.5 animate-pulse rounded-full bg-muted-foreground"
          style={{ animationDelay: `${String(delay)}ms` }}
        />
      ))}
    </span>
  );
}

function UserText({ text, names }: { readonly text: string; readonly names: readonly string[] }) {
  return (
    <>
      {mentionSegments(text, names).map((segment, index) =>
        segment.mention ? (
          <span key={index} className="font-semibold">
            {segment.text}
          </span>
        ) : (
          <Fragment key={index}>{segment.text}</Fragment>
        ),
      )}
    </>
  );
}

export function GroupMessageRow({
  groupId,
  message,
  names,
  onOpenAgent,
}: {
  readonly groupId: string;
  readonly message: AssistantGroupMessage;
  readonly names: readonly string[];
  readonly onOpenAgent: (agentId: string) => void;
}) {
  if (message.role === "notice") {
    return (
      <div className="min-w-0 px-8">
        <p role="note" aria-label="Group notice" className="text-center text-11 text-text-tertiary">{message.text}</p>
          {message.activities?.filter((activity) => activity.machine).map((activity) => (
            <MachineToolCard key={activity.id} receipt={activity.machine!} />
          ))}
      </div>
    );
  }
  if (message.role === "user") {
    return (
      <div className="ml-[30px] flex justify-end">
        <div className="max-w-[78%] whitespace-pre-wrap break-words rounded-lg bg-overlay-strong px-3 py-2 text-12 leading-relaxed text-foreground">
          {message.author ? <p className="mb-1 text-11 font-medium text-muted-foreground">{message.author.display_name}</p> : null}
          <UserText text={message.text} names={names} />
          {message.attachments?.map((item) => (
            <ToolImage
              key={item.id}
              image={{
                id: item.id,
                label: item.label,
                contentType: item.content_type,
                imageInput: item.image_input ?? undefined,
                expired: item.expired,
                endpoint: `/assistant/nyxagent/groups/${groupId}/attachments/${item.id}`,
              }}
            />
          ))}
        </div>
      </div>
    );
  }
  const agent = message.agent ?? { id: "unknown", name: "Agent", kind: "specialist" as const };
  const title = agentTitle(agent);
  const handle = agentHandle(agent);
  const time = formatClockTime(message.created_at);
  return (
    <article
      aria-label={`Message from ${title}`}
      className="grid grid-cols-[30px_minmax(0,1fr)] items-start"
    >
      <button
        type="button"
        tabIndex={-1}
        aria-hidden="true"
        onClick={() => onOpenAgent(agent.id)}
        className="mt-0.5 flex w-6 justify-center rounded-full"
      >
        <AgentAvatar agent={agent} size="md" />
      </button>
      <div className="min-w-0">
        <div className="mb-0.5 flex items-baseline gap-2">
          <span className="text-12 font-medium text-foreground">{title}</span>
          {handle ? <span className="text-11 text-text-tertiary">{handle}</span> : null}
          {agent.kind === "nyxbot" ? (
            <span className="text-10 text-text-tertiary">Personal agent</span>
          ) : null}
          {time ? (
            <time dateTime={message.created_at} className="font-mono text-11 text-text-tertiary">
              {time}
            </time>
          ) : null}
        </div>
        <div className="px-px text-foreground">
          {message.activities?.filter((activity) => activity.machine).map((activity) => (
            <MachineToolCard key={activity.id} receipt={activity.machine!} />
          ))}
          <TextBlock text={sanitizeAssistantMessageContent(message.text)} />
        </div>
      </div>
    </article>
  );
}

/** The group's messages; follows new messages while the reader is at the bottom. */
function GroupTranscript({
  group,
  messages,
  hasOlder,
  onLoadOlder,
  onOpenAgent,
  bottomInset,
}: {
  readonly group: AssistantGroup;
  readonly messages: readonly AssistantGroupMessage[];
  readonly hasOlder: boolean;
  readonly onLoadOlder: () => Promise<void>;
  readonly onOpenAgent: (agentId: string) => void;
  readonly bottomInset: number;
}) {
  const scrollRef = useRef<HTMLDivElement>(null);
  const following = useRef(true);
  const [loadingOlder, setLoadingOlder] = useState(false);
  const names = group.members.map((member) => member.name);
  const working = group.members.filter((member) =>
    group.working_agent_ids.includes(member.id),
  );
  const lastSeq = messages.at(-1)?.seq;

  useLayoutEffect(() => {
    const element = scrollRef.current;
    if (!element || !following.current) return;
    element.scrollTop = element.scrollHeight;
  }, [lastSeq, working.length, bottomInset]);

  function handleScroll(event: UIEvent<HTMLDivElement>) {
    const element = event.currentTarget;
    following.current = element.scrollHeight - element.clientHeight - element.scrollTop <= 48;
  }

  async function loadOlder() {
    const element = scrollRef.current;
    const before = element ? element.scrollHeight - element.scrollTop : 0;
    setLoadingOlder(true);
    try {
      await onLoadOlder();
    } catch {
      toast.error("Could not load earlier messages. Try again.");
    } finally {
      setLoadingOlder(false);
      // Keep the reader's place once older messages are prepended.
      requestAnimationFrame(() => {
        if (element) element.scrollTop = element.scrollHeight - before;
      });
    }
  }

  return (
    <div
      ref={scrollRef}
      onScroll={handleScroll}
      className="assistant-scrollbar min-h-0 flex-1 overflow-y-auto px-4 sm:px-6"
    >
      <div
        className="mx-auto flex min-h-full w-full max-w-[758px] flex-col gap-4 pt-4"
        style={{ paddingBottom: Math.max(bottomInset + 24, 96) }}
      >
        {hasOlder ? (
          <Button
            variant="ghost"
            size="sm"
            className="self-center"
            isLoading={loadingOlder}
            onClick={() => void loadOlder()}
          >
            Load earlier messages
          </Button>
        ) : null}
        {!messages.length ? (
          <p className="flex flex-1 items-center justify-center px-6 text-center text-12 text-text-tertiary">
            Say hello. Mention an agent with @ to ask it directly.
          </p>
        ) : null}
        {messages.map((message) => (
          <GroupMessageRow
            key={message.id}
            message={message}
            groupId={group.id}
            names={names}
            onOpenAgent={onOpenAgent}
          />
        ))}
        {working.length ? (
          <div
            role="status"
            aria-label="Agents working"
            className="grid grid-cols-[30px_minmax(0,1fr)] items-center"
          >
            <span className="flex w-6 justify-center">
              <AgentAvatar agent={working[0]!} size="md" />
            </span>
            <span className="flex items-center gap-2 text-11 text-muted-foreground">
              <WorkingDots />
              {workingLabel(working.map((member) => agentTitle(member)))}
            </span>
          </div>
        ) : null}
      </div>
    </div>
  );
}

function MemberButton({
  member,
  onOpen,
}: {
  readonly member: AssistantGroupMember;
  readonly onOpen: () => void;
}) {
  return (
    <button
      type="button"
      onClick={onOpen}
      aria-label={`${agentTitle(member)}${member.destroyed ? " (destroyed)" : ""} — agent details`}
      title={agentHandle(member) ? `${agentTitle(member)} (${agentHandle(member)!})` : agentTitle(member)}
      className={cn(
        "relative rounded-full transition-opacity hover:opacity-80",
        member.destroyed && "opacity-50",
      )}
    >
      <AgentAvatar agent={member} size="md" />
      {member.working ? (
        <span
          aria-hidden="true"
          className="absolute -bottom-0.5 -right-0.5 h-2 w-2 animate-pulse rounded-full bg-success ring-2 ring-background"
        />
      ) : null}
    </button>
  );
}

function GroupHeader({
  group,
  onOpenAgent,
  onOpenSettings,
}: {
  readonly group: AssistantGroup;
  readonly onOpenAgent: (agentId: string) => void;
  readonly onOpenSettings: () => void;
}) {
  const lead = group.members.find((member) => member.id === group.lead_agent_id);
  const working = group.working_agent_ids.length;
  return (
    <div className="shrink-0 px-4 pt-3 sm:px-6">
      <div className="mx-auto w-full max-w-[758px] space-y-1.5">
        <div className="flex items-center gap-3">
          <h2 className="min-w-0 truncate text-13 font-semibold text-foreground">
            {group.name}
          </h2>
          <div aria-label="Members" role="group" className="flex items-center gap-1">
            {group.members.map((member) => (
              <MemberButton
                key={member.id}
                member={member}
                onOpen={() => onOpenAgent(member.id)}
              />
            ))}
          </div>
          {working ? (
            <span className="flex items-center gap-1.5 text-11 text-muted-foreground">
              <span aria-hidden="true" className="h-1.5 w-1.5 animate-pulse rounded-full bg-success" />
              {working} working
            </span>
          ) : null}
          <Button
            size="sm"
            variant="ghost"
            className="ml-auto"
            aria-label="Group settings"
            onClick={onOpenSettings}
          >
            <Settings2 aria-hidden="true" />
            Settings
          </Button>
        </div>
        <p className="text-11 text-text-tertiary">
          {lead ? `Messages go to ${agentTitle(lead)}` : "Messages go to the first agent"} unless you
          @mention someone. Agents hand work to each other the same way.
        </p>
      </div>
    </div>
  );
}

/** A group chat: the user plus several agents, in one transcript. */
/**
 * Members' actions waiting for the owner. A group cannot show confirmation
 * cards inline, so answering posts the card's phrase ("yes 1234" / "no 1234")
 * to the group: NyxID decides the card and sends the member back to it.
 */
export function GroupPendingActions({
  actions,
  members,
  sending,
  onAnswer,
  onDecide,
}: {
  readonly actions: readonly AssistantGroupPendingAction[];
  readonly members: readonly { readonly id: string; readonly name: string; readonly display_name?: string | null }[];
  readonly sending: boolean;
  readonly onAnswer: (text: string) => Promise<void>;
  readonly onDecide?: (action: AssistantGroupPendingAction, decision: "allow" | "deny") => Promise<void>;
}) {
  if (!actions.length) return null;
  return (
    // Opaque and above the composer's fade: the transcript scrolls
    // underneath, and the fade must not cover these cards' buttons.
    <div className="relative z-[1] bg-background">
      <div
        aria-hidden="true"
        className="pointer-events-none absolute inset-x-0 bottom-full h-6 bg-gradient-to-t from-background to-transparent"
      />
      <div className="mx-auto w-full max-w-3xl space-y-1.5 px-4 pb-2" aria-label="Actions waiting for you">
        {actions.map((action) => {
          const member = members.find((candidate) => candidate.id === action.agent_id);
          const who = member?.display_name ?? member?.name ?? "An agent";
          const code = action.confirm_phrase?.slice(4);
          return (
            <div
              key={action.acknowledgement_id ?? action.approval_request_id}
              role="region"
              aria-label={`Confirm: ${action.summary}`}
              className="flex items-center gap-3 rounded-lg border border-border bg-overlay px-3 py-2"
            >
              <p className="min-w-0 flex-1 text-12 text-foreground">
                <span className="font-medium">{who}</span> wants to: {action.summary}
              </p>
              {action.triggering_person ? <>
                <span className="text-11 text-muted-foreground">Awaiting {action.triggering_person.display_name}</span>
                {action.can_decide ? <>
                  <Button size="sm" variant="outline" disabled={sending} onClick={() => void onDecide?.(action, "deny")}>Deny</Button>
                  <Button size="sm" disabled={sending} onClick={() => void onDecide?.(action, "allow")}>Allow</Button>
                </> : null}
              </> : action.confirm_phrase ? <>
              <Button
                size="sm"
                variant="outline"
                disabled={sending}
                onClick={() => void onAnswer(`no ${code}`)}
              >
                Cancel
              </Button>
              <Button size="sm" disabled={sending} onClick={() => void onAnswer(action.confirm_phrase!)}>
                Confirm
              </Button>
              </> : null}
            </div>
          );
        })}
      </div>
    </div>
  );
}

export function NyxAgentGroupPage({
  groupId,
  mock,
}: {
  readonly groupId: string;
  readonly mock: boolean;
}) {
  const navigate = useNavigate();
  const user = useAuthStore((state) => state.user);
  const agents = useNyxBotAgents();
  const transcript = useNyxBotGroupMessages(groupId);
  const [deciding, setDeciding] = useState(false);
  const decideApproval = useDecideApproval();
  const composerRef = useRef<HTMLDivElement>(null);
  const [composerHeight, setComposerHeight] = useState(0);
  const [detailsAgentId, setDetailsAgentId] = useState<string>();
  const [settingsOpen, setSettingsOpen] = useState(false);
  // Group payloads name agents by handle; their display names live on the agent list.
  const group = transcript.data && !transcript.error
    ? groupWithDisplayNames(transcript.data.group, agents.data?.agents)
    : undefined;
  const messages = (transcript.error ? [] : transcript.data?.messages ?? []).map((message) =>
    message.agent ? { ...message, agent: withDisplayName(message.agent, agents.data?.agents) } : message,
  );

  useLayoutEffect(() => {
    const element = composerRef.current;
    if (!element || typeof ResizeObserver === "undefined") return;
    const observer = new ResizeObserver((entries) => {
      setComposerHeight(entries[0]?.contentRect.height ?? 0);
    });
    observer.observe(element);
    return () => observer.disconnect();
  }, []);

  function goTo(search: { c?: string } = {}) {
    void navigate({
      to: "/assistant" as never,
      search: { ...search, ...(mock ? { mock: 1 } : {}) } as never,
    });
  }

  const mentions = (group?.members ?? [])
    .filter((member) => !member.destroyed)
    .map((member) => ({
      id: member.id,
      name: member.name,
      kind: member.kind,
      display_name: withDisplayName(member, agents.data?.agents).display_name,
    }));

  return (
    <AssistantShell
      title={group?.name ?? "Group"}
      headerActions={<NyxBotSettingsButton />}
      sidebar={
        <AssistantEngineSidebar
          engine="nyxagent"
          conversations={[]}
          activeConversationId={undefined}
          onNewChat={() => goTo()}
          onSelect={(id) => goTo({ c: id })}
          onDelete={(id) => nyxAgentTransport.delete(id)}
        />
      }
    >
      <div className="relative flex h-full min-h-0 flex-col bg-background">
        {group ? (
          <>
            <GroupHeader
              group={group}
              onOpenAgent={setDetailsAgentId}
              onOpenSettings={() => setSettingsOpen(true)}
            />
            <AssistantLinkModalHost>
              <GroupTranscript
                group={group}
                messages={messages}
                hasOlder={Boolean(transcript.data?.before_seq)}
                onLoadOlder={transcript.loadOlder}
                onOpenAgent={setDetailsAgentId}
                bottomInset={composerHeight}
              />
            </AssistantLinkModalHost>
            {settingsOpen ? (
              <GroupSettingsDialog
                group={group}
                agents={agents.data?.agents ?? []}
                open={settingsOpen}
                onOpenChange={setSettingsOpen}
                onDeleted={() => {
                  setSettingsOpen(false);
                  goTo();
                }}
              />
            ) : null}
          </>
        ) : transcript.error ? (
          <div className="flex flex-1 flex-col items-center justify-center gap-3 px-6 text-center">
            <p className="text-12 text-muted-foreground">
              Could not open this group. {transcript.error.message}
            </p>
            <Button variant="outline" size="sm" onClick={() => goTo()}>
              Back to home
            </Button>
          </div>
        ) : (
          <div className="flex flex-1 items-center justify-center text-12 text-text-tertiary">
            Loading group...
          </div>
        )}
        <AgentDetailsSheet
          agentId={detailsAgentId}
          agents={agents.data?.agents ?? []}
          open={detailsAgentId !== undefined}
          onOpenChange={(open) => {
            if (!open) setDetailsAgentId(undefined);
          }}
          onDeleted={() => setDetailsAgentId(undefined)}
        />
        <div ref={composerRef} className="absolute inset-x-0 bottom-0 z-10">
          <GroupPendingActions
            actions={transcript.error ? [] : transcript.data?.pending_actions ?? []}
            members={group?.members ?? []}
            sending={transcript.post.isPending || deciding}
            onDecide={async (action, decision) => {
              setDeciding(true);
              try {
                if (action.approval_request_id) {
                  await decideApproval.mutateAsync({ requestId: action.approval_request_id, approved: decision === "allow" });
                } else if (action.acknowledgement_id) {
                  await nyxAgentTransport.decide(action.conversation_id, action.acknowledgement_id, decision);
                }
                await transcript.refetch();
              } catch (error) { toast.error(error instanceof Error ? error.message : "Could not decide this action."); }
              finally { setDeciding(false); }
            }}
            onAnswer={async (text) => {
              try {
                await transcript.post.mutateAsync(text);
              } catch (error) {
                toast.error(
                  error instanceof Error ? error.message : "The answer was not delivered.",
                );
              }
            }}
          />
          <UploadComposer
            key={`${user?.id}:${groupId}`}
            scope={{ kind: "groups", id: groupId }}
            active={false}
            sending={transcript.post.isPending}
            // Read-only once every member is gone (destroyed or removed).
            disabled={Boolean(transcript.error) || Boolean(group && !mentions.length)}
            ownerUserId={user?.id ?? null}
            draftKey={`group:${groupId}`}
            placeholder={GROUP_COMPOSER_PLACEHOLDER}
            mentions={mentions}
            onSend={async (text, uploads) => {
              try {
                await transcript.post.mutateAsync(
                  uploads ? { text, attachmentIds: uploads.attachmentIds } : text,
                );
              } catch (error) {
                toast.error(
                  error instanceof Error ? error.message : "The message was not delivered.",
                );
                throw error;
              }
            }}
            onStop={async () => undefined}
          />
        </div>
      </div>
    </AssistantShell>
  );
}
