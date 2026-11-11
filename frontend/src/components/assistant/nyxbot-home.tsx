import { useUiTranslation } from "@/i18n/use-ui-translation";
import { translateUi } from "@/i18n/use-ui-translation";
import { agentOwnerSections } from "@/lib/assistant/nyxbot-labels";
import { useState, type ReactNode } from "react";
import { MessageSquare, NotebookPen, Plus } from "lucide-react";
import { AgentAvatar, AgentAvatarStack } from "@/components/assistant/nyxbot-agent-avatar";
import { AgentStatusDot } from "@/components/assistant/nyxbot-agent-panels";
import { Button } from "@/components/ui/button";
import { nyxBotOf, useNyxBotAgent } from "@/hooks/use-nyxbot-agents";
import { AWAY_PREVIEW, awayItems } from "@/lib/assistant/nyxbot-away";
import {
  AGENT_STATUS_LABEL,
  agentHandle,
  agentTitle,
  greetingFor,
} from "@/lib/assistant/nyxbot-labels";
import { formatRelativeTime } from "@/lib/utils";
import type { AssistantAgent, AssistantGroup } from "@/schemas/assistant-nyxagent";
import type { NextStep } from "@/schemas/assistant-onboarding";
import { NextStepCards } from "./onboarding/next-step-cards";

const MEMORY_PREVIEW = 3;


function Section({
  id,
  title,
  action,
  children,
}: {
  readonly id: string;
  readonly title: string;
  readonly action?: ReactNode;
  readonly children: ReactNode;
}) {
  useUiTranslation();
  return (
    <section aria-labelledby={id} data-tour={id} className="space-y-3">
      <div className="flex items-center justify-between gap-3">
        <h2 id={id} className="type-card-title text-foreground">
          {title}
        </h2>
        {action}
      </div>
      {children}
    </section>
  );
}

function Quiet({ children }: { readonly children: ReactNode }) {
  useUiTranslation();
  return (
    <p className="rounded-lg border border-dashed border-border px-4 py-3 text-14 text-muted-foreground">{children}</p>
  );
}

/**
 * The home's list card, shared by "While you were away" and "What NyxBot
 * remembers": a lead (avatar or icon), a title line, two lines of body and an
 * optional action per row, then "and N more".
 */
function HomeList({
  label,
  more,
  children,
}: {
  readonly label?: string;
  readonly more?: ReactNode;
  readonly children: ReactNode;
}) {
  useUiTranslation();
  return (
    <div className="overflow-hidden rounded-xl border border-border/50 bg-card">
      <ul aria-label={label}>{children}</ul>
      {more ? <div className="px-4 pb-3 text-13">{more}</div> : null}
    </div>
  );
}

function HomeListRow({
  lead,
  title,
  body,
  action,
}: {
  readonly lead: ReactNode;
  readonly title: ReactNode;
  readonly body: ReactNode;
  readonly action?: ReactNode;
}) {
  useUiTranslation();
  return (
    <li className="flex items-start gap-3 px-4 py-3">
      {lead}
      <div className="min-w-0 flex-1 space-y-0.5">
        <div className="flex items-center gap-2 text-14">{title}</div>
        <p className="line-clamp-2 text-14 text-muted-foreground">{body}</p>
      </div>
      {action}
    </li>
  );
}

function MoreButton({
  onClick,
  disabled,
  children,
}: {
  readonly onClick: () => void;
  readonly disabled?: boolean;
  readonly children: ReactNode;
}) {
  useUiTranslation();
  return (
    <button
      type="button"
      onClick={onClick}
      disabled={disabled}
      className="text-text-tertiary underline-offset-2 transition-colors hover:text-foreground hover:underline focus-visible:underline focus-visible:outline-none"
    >
      {children}
    </button>
  );
}

export function AwayList({
  agents,
  onOpenAgent,
  onOpenConversation,
}: {
  readonly agents: readonly AssistantAgent[];
  readonly onOpenAgent: (agent: AssistantAgent) => void;
  readonly onOpenConversation: (conversationId: string) => void;
}) {
  useUiTranslation();
  const items = awayItems(agents);
  const [shown, setShown] = useState(AWAY_PREVIEW);
  if (!items.length) {
    return (
      <Quiet>{translateUi("All caught up. Replies and requests from your specialists show up here.")}</Quiet>
    );
  }
  return (
    <HomeList
      more={items.length > AWAY_PREVIEW ? (
        <MoreButton onClick={() => setShown(shown < items.length ? shown + AWAY_PREVIEW : AWAY_PREVIEW)}>
          {shown < items.length ? translateUi("and {{value1}} more", { value1: items.length - shown }) : translateUi("Show less")}
        </MoreButton>
      ) : null}
    >
      {items.slice(0, shown).map((item) => (
        <HomeListRow
          key={item.key}
          lead={<AgentAvatar agent={item.agent} size="lg" />}
          title={
            <>
              <span className="font-medium text-foreground">{agentTitle(item.agent)}</span>
              {item.kind === "request" ? (
                <span className="rounded-md border border-warning/30 bg-warning/10 px-1.5 type-group-label text-warning light:border-transparent light:bg-warning light:text-white">{translateUi("Needs a decision")}</span>
              ) : (
                <span className="flex items-center gap-1.5 text-13 text-text-tertiary">
                  <AgentStatusDot status={item.agent.status} />
                  {translateUi(AGENT_STATUS_LABEL[item.agent.status])}
                  <span aria-hidden="true">·</span>
                  {formatRelativeTime(item.at)}
                </span>
              )}
            </>
          }
          body={item.kind === "request" ? item.request.summary : item.text}
          action={item.kind === "request" ? (
            <Button
              size="sm"
              variant="outline"
              aria-label={translateUi("Review {{value1}}'s request: {{value2}}", { value1: agentTitle(item.agent), value2: item.request.summary })}
              onClick={() => onOpenConversation(item.request.conversation_id)}
            >{translateUi("Review")}</Button>
          ) : (
            <Button
              size="sm"
              variant="ghost"
              aria-label={translateUi("Open {{value1}}", { value1: agentTitle(item.agent) })}
              onClick={() => onOpenAgent(item.agent)}
            >{translateUi("Open")}</Button>
          )}
        />
      ))}
    </HomeList>
  );
}

function MemoryPreview({
  nyxbot,
  onManage,
}: {
  readonly nyxbot: AssistantAgent | undefined;
  readonly onManage: () => void;
}) {
  useUiTranslation();
  const detail = useNyxBotAgent(nyxbot?.id);
  const notes = detail.data?.memory ?? [];
  return (
    <Section
      id="nyxbot-home-memory"
      title={translateUi("What NyxBot remembers")}
      action={
        <Button size="sm" variant="ghost" onClick={onManage} disabled={!nyxbot}>{translateUi("Manage")}</Button>
      }
    >
      {detail.isPending && nyxbot ? (
        <Quiet>{translateUi("Loading memory...")}</Quiet>
      ) : notes.length ? (
        <HomeList
          label={translateUi("NyxBot memory preview")}
          more={notes.length > MEMORY_PREVIEW ? (
            <MoreButton onClick={onManage} disabled={!nyxbot}>{translateUi("and ")}{notes.length - MEMORY_PREVIEW}{translateUi(" more")}</MoreButton>
          ) : null}
        >
          {notes.slice(0, MEMORY_PREVIEW).map((note) => (
            <HomeListRow
              key={note.id}
              lead={
                <span className="flex size-8 shrink-0 items-center justify-center rounded-full border border-hairline bg-overlay text-text-tertiary">
                  <NotebookPen aria-hidden="true" className="size-3.5" />
                </span>
              }
              title={
                <>
                  <span className="font-medium text-foreground">{translateUi("Remembered")}</span>
                  <span className="text-13 text-text-tertiary">{formatRelativeTime(note.updated_at)}</span>
                </>
              }
              body={note.text}
            />
          ))}
        </HomeList>
      ) : (
        <Quiet>{translateUi("Nothing yet. Tell NyxBot “Remember …” and it keeps that across every chat.")}</Quiet>
      )}
    </Section>
  );
}

function RosterCard({
  agent,
  onChat,
}: {
  readonly agent: AssistantAgent;
  readonly onChat: (agent: AssistantAgent) => void;
}) {
  useUiTranslation();
  const nyxbot = agent.kind === "nyxbot";
  const handle = agentHandle(agent);
  return (
    <li className="flex flex-col gap-3 rounded-xl border border-border/50 bg-card p-4">
      <div className="flex items-start gap-3">
        <AgentAvatar agent={agent} size="lg" />
        <div className="min-w-0 flex-1">
          <p className="flex min-w-0 items-baseline gap-1.5">
            <span className="truncate type-card-title text-foreground">
              {agentTitle(agent)}
            </span>
            {handle ? (
              <span className="shrink-0 text-13 text-text-tertiary">
                {handle}
              </span>
            ) : null}
          </p>
          <p className="flex items-center gap-1.5 text-13 text-text-tertiary">
            {nyxbot ? (
              translateUi("Your personal agent")
            ) : (
              <>
                <AgentStatusDot status={agent.status} />
                {translateUi(AGENT_STATUS_LABEL[agent.status])}
                {agent.pending_requests.length ? (
                  <span className="text-warning">
                    · {agent.pending_requests.length}{" "}
                    {agent.pending_requests.length === 1 ? translateUi("request") : translateUi("requests")}
                  </span>
                ) : null}
              </>
            )}
          </p>
        </div>
      </div>
      <p className="line-clamp-2 min-h-8 text-14 text-muted-foreground">
        {agent.description ||
          (nyxbot
            ? translateUi("Full access to your services and account. Delegates to your specialists.")
            : translateUi("No role described yet."))}
      </p>
      <div className="flex justify-end">
        <Button
          size="sm"
          variant="outline"
          aria-label={translateUi("Chat with {{value1}}", { value1: agentTitle(agent) })}
          disabled={agent.can_use === false}
          onClick={() => onChat(agent)}
        >
          <MessageSquare aria-hidden="true" />{translateUi("Chat")}</Button>
      </div>
    </li>
  );
}

function GroupCard({
  group,
  onOpen,
}: {
  readonly group: AssistantGroup;
  readonly onOpen: () => void;
}) {
  useUiTranslation();
  const working = group.working_agent_ids.length;
  return (
    <li>
      <button
        type="button"
        onClick={onOpen}
        aria-label={translateUi("Open crew {{value1}}", { value1: group.name })}
        className="flex w-full flex-col gap-2.5 rounded-xl border border-border/50 bg-card p-4 text-left transition-colors hover:bg-overlay"
      >
        <div className="flex w-full items-center gap-2">
          <AgentAvatarStack agents={group.members} size="sm" max={4} />
          {working ? (
            <span className="ml-auto flex items-center gap-1.5 text-13 text-muted-foreground">
              <span aria-hidden="true" className="h-1.5 w-1.5 animate-pulse rounded-full bg-success" />
              {working}{translateUi(" working")}</span>
          ) : null}
        </div>
        <span className="truncate type-card-title text-foreground">
          {group.name}
        </span>
        <span className="truncate text-13 text-text-tertiary">
          {group.owner?.type === "org" ? translateUi("{{name}} · ", { name: group.owner.name }) : ""}
          {group.members.map((member) => agentTitle(member)).join(", ")}
          {group.last_message_at ? translateUi(" · {{value}}", { value: formatRelativeTime(group.last_message_at) }) : ""}
        </span>
      </button>
    </li>
  );
}

/**
 * The NyxBot home: a greeting, what happened while the user was away, what
 * NyxBot remembers, the roster and the crews. The page renders the
 * "Message NyxBot" composer below it.
 */
export function NyxBotHome({
  userName,
  agents,
  agentsLoading,
  groups,
  bottomInset,
  onChat,
  onOpenConversation,
  onOpenGroup,
  onNewGroup,
  onNewAgent,
  onManageMemory,
  nextSteps,
  onStartNextStep,
  onHideNextSteps,
  onReplayIntro,
}: {
  readonly userName: string | undefined;
  readonly agents: readonly AssistantAgent[];
  readonly agentsLoading: boolean;
  readonly groups: readonly AssistantGroup[];
  readonly bottomInset: number;
  readonly onChat: (agent: AssistantAgent) => void;
  readonly onOpenConversation: (conversationId: string) => void;
  readonly onOpenGroup: (groupId: string) => void;
  readonly onNewGroup: () => void;
  readonly onNewAgent: () => void;
  readonly onManageMemory: () => void;
  /** Suggested next steps; the section is hidden when there are none. */
  readonly nextSteps?: readonly NextStep[];
  readonly onStartNextStep?: (step: NextStep) => void;
  /** Hides the whole section until the intro is retaken. */
  readonly onHideNextSteps?: () => void;
  readonly onReplayIntro?: () => void;
}) {
  useUiTranslation();
  const [now] = useState(() => new Date());
  const firstName = userName?.trim().split(/\s+/)[0];
  const nyxbot = nyxBotOf(agents);
  const roster = agents
    .filter((agent) => agent.status !== "destroyed")
    .sort((a, b) => Number(b.kind === "nyxbot") - Number(a.kind === "nyxbot"));
  return (
    <div data-chat-transcript="" data-scroll-fade="" className="assistant-scrollbar min-h-0 flex-1 overflow-x-hidden overflow-y-auto overscroll-contain px-4 sm:px-6">
      <div
        className="mx-auto flex w-full max-w-[var(--chat-content-width,758px)] flex-col gap-8 pt-8"
        style={{ paddingBottom: Math.max(bottomInset + 24, 96) }}
      >
        <header className="flex items-center gap-4">
          <AgentAvatar agent={{ id: "nyxbot", name: "NyxBot", kind: "nyxbot" }} size="lg" />
          <div className="min-w-0 space-y-1">
            <h1
              className="type-compact-page-title text-foreground sm:type-page-title"
              style={{ letterSpacing: "-0.03em" }}
            >
              {translateUi(greetingFor(now))}
              {firstName ? `, ${firstName}` : ""}
            </h1>
            <p className="text-14 text-muted-foreground">{translateUi("NyxBot is your personal agent. Ask it anything below, or check in on your team.")}</p>
          </div>
        </header>

        {nextSteps?.length && onStartNextStep ? (
          <Section
            id="nyxbot-home-next-steps"
            title={translateUi("Suggested for you")}
            action={
              <div className="flex items-center gap-1">
                {onReplayIntro ? (
                  <Button size="sm" variant="ghost" onClick={onReplayIntro}>{translateUi("Replay intro")}</Button>
                ) : null}
                {onHideNextSteps ? (
                  <Button size="sm" variant="ghost" onClick={onHideNextSteps}>{translateUi("Hide")}</Button>
                ) : null}
              </div>
            }
          >
            <NextStepCards
              steps={nextSteps}
              onStart={onStartNextStep}
              className="sm:grid-cols-2"
            />
          </Section>
        ) : null}

        <Section id="nyxbot-home-away" title={translateUi("While you were away")}>
          {agentsLoading ? (
            <Quiet>{translateUi("Loading your agents...")}</Quiet>
          ) : (
            <AwayList
              agents={agents}
              onOpenAgent={onChat}
              onOpenConversation={onOpenConversation}
            />
          )}
        </Section>

        <MemoryPreview nyxbot={nyxbot} onManage={onManageMemory} />

        <Section
          id="nyxbot-home-agents"
          title={translateUi("Your agents")}
          action={
            <Button size="sm" variant="ghost" onClick={onNewAgent}>
              <Plus aria-hidden="true" />{translateUi("New agent")}</Button>
          }
        >
          {roster.length ? (
            <div aria-label={translateUi("Agents")} className="space-y-4">
              {agentOwnerSections(roster).map((section) => (
                <section
                  key={section.id}
                  aria-label={section.label}
                  className="space-y-2"
                >
                  <h3 className="type-table-header uppercase tracking-[1.5px] text-text-tertiary">
                    {section.label}
                  </h3>
                  <ul className="grid gap-3 sm:grid-cols-2">
              {section.agents.map((agent) => (
                <RosterCard key={agent.id} agent={agent} onChat={onChat} />
              ))}
            </ul>
                </section>
              ))}
            </div>
          ) : (
            <Quiet>
              {agentsLoading ? translateUi("Loading your agents...") : translateUi("No agents yet.")}
            </Quiet>
          )}
        </Section>

        <Section
          id="nyxbot-home-groups"
          title={translateUi("Crews")}
          action={
            <Button size="sm" variant="ghost" onClick={onNewGroup}>
              <Plus aria-hidden="true" />{translateUi("New crew")}</Button>
          }
        >
          {groups.length ? (
            <ul aria-label={translateUi("Crews")} className="grid gap-3 sm:grid-cols-2">
              {groups.map((group) => (
                <GroupCard key={group.id} group={group} onOpen={() => onOpenGroup(group.id)} />
              ))}
            </ul>
          ) : (
            <Quiet>{translateUi("Put several agents in one chat. Mention one with @ to ask it directly; NyxBot answers the rest.")}</Quiet>
          )}
        </Section>
      </div>
    </div>
  );
}
