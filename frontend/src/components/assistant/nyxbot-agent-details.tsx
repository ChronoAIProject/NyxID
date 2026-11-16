import { useTriggers } from "@/hooks/use-triggers";
import { agentDetailsSearchTerms } from "@/lib/settings-search";
import { useSectionSearchParam } from "@/hooks/use-section-search-param";
import { SectionPages } from "@/components/ui/section-pages";
import { CheckmarkList } from "@/components/ui/checkmark-list";
import { AGENT_DETAILS_SECTIONS, AGENT_DETAILS_TITLES, AGENT_PROFILE_FIELDS, AGENT_NOTES_LABEL, agentDetailsAvailability, agentDetailsSections, type AgentDetailsSection } from "@/lib/assistant/agent-details-sections";
import { useRouter } from "@tanstack/react-router";
import { UserRound, ShieldCheck, MessageSquare, AudioLines, CalendarClock, Wrench, Users, Monitor, Plug, BookOpen, Brain, BadgeInfo, MessageCircle, ListChecks } from "lucide-react";
import { OrgGuestGrantsAdoption } from "./org-guest-grants-adoption";
import { useUiTranslation } from "@/i18n/use-ui-translation";
import { translateUi } from "@/i18n/use-ui-translation";
import { Label } from "@/components/ui/label";
import { useKeys } from "@/hooks/use-keys";
import { useNodes } from "@/hooks/use-nodes";
import { useSavedLogins } from "@/hooks/use-saved-logins";
import { describeChanges, changeList, labelValue, recordChanges, type ChangeField } from "@/lib/grouped-form-changes";
import { DrawerSectionInfoContext } from "@/lib/drawer-section-info";
import { useCompactTitle } from "@/lib/compact-title";
import { CompactTitleProvider, CompactChrome, CompactScroll } from "@/components/ui/compact-title";
import { FormViewport } from "@/components/ui/form-viewport";
import { useVisualViewportBounds } from "@/lib/form-viewport";
import { DrawerSearchProvider, AgentDrawerSearch } from "./agent-drawer-search";
import { DrawerSearchTabContext, type DrawerSearchTab } from "@/lib/drawer-search";
import {
  GroupedFormActions,
  GroupedFormScope,
  GroupedFormBar,
} from "@/components/ui/grouped-form";
import { GroupedDetailContext } from "@/lib/grouped-detail";
import { GroupedDraftBoundary, GroupedDraftDialog, GroupedDraftNavigation } from "@/components/ui/grouped-drafts";
import { useGroupedDraftGuard } from "@/lib/grouped-drafts";
import { CollaborationGuestControls } from "./collaboration-guest-controls";
import { GuestReadiness } from "./guest-readiness";
import { ShareAgentDialog, SharedAgentMemory, SharedAgentProfile, SharedConsentCards } from "./shared-agent-controls";
import { AgentSkills } from "./agent-skills";
import { AgentConnections } from "./agent-connections";
import { AgentVoiceService } from "./agent-voice-service";
import { AgentLearningReview } from "./agent-learning-review";
import { MachineCapabilities } from "./machine-capabilities";
import { AgentOperationScopes } from "./agent-operation-scopes";
import { MachineGrantPicker } from "./machine-grant-picker";
import { AgentAutomations } from "./automation-preferences";
import { useCallback, useEffect, useRef, useState, type ComponentProps, type ReactNode } from "react";
import { zodResolver } from "@hookform/resolvers/zod";
import { useQueryClient } from "@tanstack/react-query";
import { Trash2, X } from "lucide-react";
import {
  GroupedList,
  GroupedDisclosure,
  GroupedRow,
  GroupedInput,
  GroupedTextarea,
  GroupedSelectTrigger,
  GroupedFooter,
} from "@/components/ui/grouped-list";
import {
  SegmentedControl,
  SegmentedControlList,
  SegmentedControlTrigger,
  SegmentedControlContent,
} from "@/components/ui/segmented-control";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import {
  Form,
  FormControl,
  FormDescription,
  FormField,
  FormItem,
  FormLabel,
  FormMessage,
  useAppForm,
} from "@/components/ui/form";

import {
  Select,
  SelectContent,
  SelectItem,
  SelectValue,
} from "@/components/ui/select";
import {
  Sheet,
  SheetContent,
  SheetClose,
  SheetDescription,
  SheetHeader,
  SheetTitle,
} from "@/components/ui/sheet";
import { Switch } from "@/components/ui/switch";
import { ErrorBanner } from "@/components/shared/error-banner";
import { ChannelBotsManager } from "@/components/assistant/nyxbot-channels";
import {
  PERSONA_HINT,
  ServiceGrantPicker,
} from "@/components/assistant/nyxbot-agent-forms";
import { AgentAvatar } from "@/components/assistant/nyxbot-agent-avatar";
import {
  nyxBotQueryKeys,
  useDeleteNyxBotAgent,
  useDestroyNyxBotAgent,
  useForgetNyxBotMemory,
  useRememberNyxBotMemory,
  useNyxBotAgent,
  useGuestReadiness,
  useNyxBotSettings,
  useNyxAgentModels,
  useSetNyxBotAgentGrants,
  useUpdateNyxBotAgent,
} from "@/hooks/use-nyxbot-agents";
import { ApiError } from "@/lib/api-client";
import { nyxAgentTransport } from "@/lib/assistant/nyxagent-transport";
import type { AgentDetailsTab } from "@/lib/assistant/search";
import {
  modelDisplayName,
  ACCESS_MODE_DESCRIPTION,
  ACCESS_MODE_LABEL,
  AGENT_STATUS_LABEL,
  agentHandle,
  agentTitle,
} from "@/lib/assistant/nyxbot-labels";
import { useFeature } from "@/hooks/use-feature-flag";
import { FEATURE_FLAG } from "@/lib/feature-flags";
import { ModelSelect } from "@/components/assistant/nyxbot-access-controls";
import { formatDateTime } from "@/lib/utils";
import {
  ASSISTANT_AGENT_DISPLAY_NAME_MAX,
  ASSISTANT_MEMORY_NOTE_MAX,
  assistantMemoryNoteSchema,
  ASSISTANT_AGENT_PERSONA_MAX,
  ASSISTANT_AGENT_VOICE_INSTRUCTIONS_MAX,
  assistantAgentGrantsSchema,
  assistantAgentProfileSchema,
  nyxAgentAccessModeSchema,
  type AssistantAgent,
  type AssistantAgentGrants,
  type AssistantAgentMemoryNote,
  type AssistantGuestAccess,
  type AssistantAgentProfile,
} from "@/schemas/assistant-nyxagent";
import { useAuthStore } from "@/stores/auth-store";

function errorMessage(error: unknown, fallback: string): string {
  return error instanceof Error && error.message ? error.message : fallback;
}

/**
 * Who can widen a specialist's access, and why the agent itself cannot. Org
 * agents are maintained by the organization: only its maintainers approve.
 */
export function accessAuthorityCopy(
  agent: AssistantAgent,
  requestMode: "denied" | "routed_to_nyxbot" | undefined,
): string {
  const sandbox = "This specialist is sandboxed: it cannot grant access to itself or to other agents.";
  const direct =
    "When you talk to it directly, it tells you what it is missing and continues; you grant access here.";
  if (agent.owner_kind === "org") {
    const org = agent.owner_name ?? "the organization";
    return agent.can_maintain === false
      ? translateUi("{{sandbox}} Only a maintainer of {{org}} (an Admin or Member) can approve or grant its access; you can view it but not change it.", { sandbox, org })
      : translateUi("{{sandbox}} Maintainers of {{org}}, including you, approve and grant its access here; your personal NyxBot cannot approve it on the organization's behalf.", { sandbox, org });
  }
  return requestMode === "routed_to_nyxbot"
    ? translateUi("{{sandbox}} {{direct}} Only for work NyxBot assigned, or in a group with NyxBot, do its requests go to NyxBot, which approves or denies them.", { sandbox, direct })
    : translateUi("{{sandbox}} {{direct}} Its requests are never routed: only work NyxBot assigned, or work in a group NyxBot is in, reports the missing access to NyxBot instead.", { sandbox, direct });
}

const GRANTS_SECTION_ID = "agent-details-grants";
const SKILLS_SECTION_ID = "agent-details-skills";

function isNoAccess(error: unknown): boolean {
  return error instanceof ApiError && (error.status === 403 || error.status === 404);
}

/** Says why the forms are read-only instead of leaving a silently dead Save button. */
function ReadOnlyNote({ agent }: { readonly agent: AssistantAgent }) {
  useUiTranslation();
  if (agent.status === "destroyed") {
    return (
      <section
        role="note"
        aria-label={translateUi("Agent destroyed")}
        className="space-y-1 px-4 py-2 text-13 text-text-tertiary"
      >
        <p className="font-medium text-foreground">
          {agentTitle(agent)}{translateUi(" was destroyed")}{agent.destroyed_at
            ? translateUi(" on {{value1}}", { value1: formatDateTime(agent.destroyed_at) })
            : ""}
          .
        </p>
        <p>{translateUi("Its grants, skills and settings are read-only and it no longer runs.")}{agent.kind === "specialist" && agent.can_maintain !== false
            ? translateUi(" You can delete it permanently below.")
            : ""}
        </p>
      </section>
    );
  }
  if (agent.share_role) return null;
  if (agent.can_maintain === false) {
    const org = agent.owner_name ?? "the organization";
    return (
      <section
        role="note"
        aria-label={translateUi("Read-only")}
        className="space-y-1 px-4 py-2 text-13 text-text-tertiary"
      >
        <p className="font-medium text-foreground">{translateUi("Read-only.")}</p>
        <p>{translateUi("Only a maintainer of ")}{org}{translateUi(" (an Admin or Member) can change its grants, skills and settings. You can view them.")}</p>
      </section>
    );
  }
  return null;
}

/**
 * Everything about one agent: its role, grants, memory, channel bots and
 * lifecycle. Opened from the thread header.
 */
export function AgentDetailsSheet(props: ComponentProps<typeof AgentDetailsSheetContent>) {
  useUiTranslation();
  return <GroupedDraftBoundary><CompactTitleProvider><AgentDetailsSheetContent {...props} /></CompactTitleProvider></GroupedDraftBoundary>;
}

function AgentDetailsSheetContent({
  agentId,
  agents,
  proposalId,
  connectionId,
  tab,
  open,
  onOpenChange,
  onDeleted,
}: {
  readonly agentId: string | undefined;
  readonly agents: readonly AssistantAgent[];
  readonly proposalId?: string;
  readonly connectionId?: string;
  readonly tab?: AgentDetailsSection;
  readonly open: boolean;
  readonly onOpenChange: (open: boolean) => void;
  readonly onDeleted: () => void;
}) {
  useUiTranslation();
  const compact = useCompactTitle();
  const viewportBounds = useVisualViewportBounds();
  const [detailContainer, setDetailContainer] = useState<HTMLDivElement | null>(
    null,
  );
  const [query, setQuery] = useState("");
  const [searchOpen, setSearchOpen] = useState(false);
  const [searchHost, setSearchHost] = useState<HTMLDivElement | null>(null);
  const session = `${agentId}:${open}`;
  const [previousSession, setPreviousSession] = useState(session);
  if (previousSession !== session) {
    setPreviousSession(session);
    setQuery("");
    setSearchOpen(false);
  }
  const detail = useNyxBotAgent(agentId, open);
  const returnFocus = useRef<HTMLElement | null>(null);
  const agent = detail.data?.agent;
  const settings = useNyxBotSettings(open);
  const requestMode = settings.data?.specialist_access_requests;
  const noAccess = detail.error ? isNoAccess(detail.error) : false;
  const guard = useGroupedDraftGuard();
  async function close() {
    if (await guard!.confirmDiscard()) onOpenChange(false);
  }
  return (
    <Sheet open={open} onOpenChange={(next) => { if (next) onOpenChange(true); else void close(); }}>
      <GroupedDraftNavigation active={open} />
      <SheetContent
        ref={setDetailContainer}
        showCloseButton={false}
        style={viewportBounds}
        onEscapeKeyDown={(event) => {
          if (query.trim()) {
            event.preventDefault();
            setQuery("");
            detailContainer?.querySelector<HTMLElement>('[role="searchbox"]')?.focus();
          } else if (searchOpen) {
            event.preventDefault();
            setSearchOpen(false);
            searchHost?.querySelector<HTMLButtonElement>('button[data-search-toggle]')?.focus();
          }
        }}
        className="agent-details-sheet flex w-full flex-col gap-0 overflow-clip bg-background p-0 sm:max-w-lg"
        onOpenAutoFocus={() => {
          returnFocus.current =
            document.activeElement instanceof HTMLElement
              ? document.activeElement
              : null;
        }}
        onCloseAutoFocus={(event) => {
          if (returnFocus.current?.isConnected) {
            event.preventDefault();
            returnFocus.current.focus({ preventScroll: true });
          }
        }}
      >
        <CompactChrome data-agent-header="" data-search-open={searchOpen} className="relative z-30 px-4 pb-2 pt-3">
          <SheetHeader className="space-y-0 text-left">
            <SheetTitle className="sr-only">{translateUi("Agent details")}</SheetTitle>
            <div className="agent-header-row relative flex min-h-8 items-center gap-2">
              {agent ? <>
                <span className="agent-header-avatar flex size-8 shrink-0 items-center justify-center"><AgentAvatar agent={agent} size="lg" /></span>
                <p title={agentTitle(agent)} className={`agent-header-name min-w-0 truncate type-card-title ${searchOpen ? "max-w-[30%] shrink" : "flex-1"}`}>{agentTitle(agent)}</p>
              </> : <span className="min-w-0 flex-1 type-card-title">{translateUi("Agent details")}</span>}
              <div ref={setSearchHost} className={searchOpen ? "min-w-0 flex-1" : "shrink-0"} />
              <SheetClose asChild>
                <button type="button" aria-label={translateUi("Close agent details")} data-icon-button="" className="agent-header-icon"><X aria-hidden className="size-4" /></button>
              </SheetClose>
            </div>
            <div className={`agent-header-subtitle pl-10 pt-1 ${compact ? "pointer-events-none absolute left-4 right-4 top-11" : ""}`} aria-hidden={compact || undefined}>
              {agent ? <>
                <SheetDescription className="text-13 text-text-tertiary">
                  {[agentHandle(agent), agent.kind === "nyxbot" ? "Personal agent" : "Specialist", AGENT_STATUS_LABEL[agent.status], `${agent.memory_count} ${agent.memory_count === 1 ? "memory" : "memories"}`].filter(Boolean).join(" · ")}
                  {agent.kind === "specialist" ? <span className="block">{translateUi("Created by ")}{agent.created_by === "nyxbot" ? translateUi("NyxBot") : translateUi("you")}</span> : null}
                </SheetDescription>
                <ReadOnlyNote agent={agent} />
              </> : <SheetDescription>{noAccess ? translateUi("This agent is not available to you.") : translateUi("Loading agent...")}</SheetDescription>}
            </div>
          </SheetHeader>
        </CompactChrome>
        {detail.error ? (
          <div className="px-5 py-4">
            {noAccess ? (
              <section
                role="note"
                aria-label={translateUi("No access")}
                className="space-y-1 text-14 text-muted-foreground"
              >
                <p className="font-medium text-foreground">{translateUi("You cannot view this agent.")}</p>
                <p>{translateUi("It belongs to an organization you are not a member of, or it no longer exists. Its grants and skills cannot be shown or changed here.")}</p>
              </section>
            ) : (
              <ErrorBanner
                message={translateUi("Could not load this agent. {{value1}}", { value1: detail.error.message })}
                onRetry={() => void detail.refetch()}
              />
            )}
          </div>
        ) : null}
        {agent ? (
          <AgentDetailsContent
            key={agent.id}
            searchHost={searchHost}
            searchOpen={searchOpen}
            setSearchOpen={setSearchOpen}
            query={query}
            setQuery={setQuery}
            agent={agent}
            summaries={!detail.error}
            agents={agents}
            proposalId={proposalId}
            connectionId={connectionId}
            tab={tab}
            requestMode={requestMode}
            memory={detail.data?.memory ?? []}
            onDeleted={onDeleted}
            onClose={() => void close()}
            detailContainer={detailContainer}
          />
        ) : null}
        <GroupedDraftDialog />
      </SheetContent>
    </Sheet>
  );
}

const DRAWER_TABS = [
  ["overview", "Overview"],
  ["grants", "Access"],
  ["skills", "Skills"],
  ["memory", "Memory"],
  ["channels", "Channels"],
] as const;

function AgentDetailsContent({
  summaries,
  searchHost,
  searchOpen,
  setSearchOpen,
  query,
  setQuery,
  agent,
  agents,
  proposalId,
  connectionId,
  tab,
  requestMode,
  memory,
  onDeleted,
  onClose,
  detailContainer,
}: {
  readonly summaries: boolean;
  readonly agent: AssistantAgent;
  readonly agents: readonly AssistantAgent[];
  readonly proposalId?: string;
  readonly connectionId?: string;
  readonly tab?: AgentDetailsSection;
  readonly requestMode?: "denied" | "routed_to_nyxbot";
  readonly searchHost: HTMLElement | null;
  readonly searchOpen: boolean;
  readonly setSearchOpen: (open: boolean) => void;
  readonly query: string;
  readonly setQuery: (query: string) => void;
  readonly memory: readonly AssistantAgentMemoryNote[];
  readonly onDeleted: () => void;
  readonly onClose: () => void;
  readonly detailContainer: HTMLDivElement | null;
}) {
  useUiTranslation();
  const searching = Boolean(query.trim());
  const orgChannels = useFeature(FEATURE_FLAG.ORG_SPECIALIST_CHANNELS);
  const voiceEnabled = useFeature(FEATURE_FLAG.VOICE_AGENT_KEY);
  const learningEnabled = useFeature(FEATURE_FLAG.AGENT_LEARNING);
  const advancedControls = useFeature(FEATURE_FLAG.ASSISTANT_MODEL_CONTROLS);
  const owner = useAuthStore((state) => state.user?.id);
  // The owner detail response already includes linked routes, including legacy org channels.
  const hasOrgChannel = agent.owner_kind === "org" && agent.channels.length > 0;
  const canManageGuests = agent.owner_kind !== "org" || (orgChannels && agent.org_role === "admin");
  const incoming: AgentDetailsSection =
    proposalId && (!tab || tab === "skills") ? "skills/learning" : (tab ?? "overview");
  const [active, setActive] = useState<DrawerSearchTab>(incoming.split("/")[0] as DrawerSearchTab);
  const panels = useRef<HTMLDivElement>(null);
  const [pageContainer, setPageContainer] = useState<HTMLDivElement | null>(null);
  const setPanels = useCallback((element: HTMLDivElement | null) => {
    panels.current = element;
    setPageContainer(element);
  }, []);
  const readOnly = agent.status === "destroyed" || agent.can_maintain === false;
  const tabs = DRAWER_TABS.filter(([value]) =>
    value === "channels"
      ? !agent.share_role &&
        (agent.owner_kind !== "org" || orgChannels || hasOrgChannel) &&
        agent.status !== "destroyed"
      : value === "memory"
        ? Boolean(agent.share_role) || agent.can_maintain !== false
        : true,
  );
  const selected = tabs.some(([value]) => value === active) ? active : "overview";
  const router = useRouter();
  const [section, setSection] = useState<AgentDetailsSection>(incoming);
  const [previousIncoming, setPreviousIncoming] = useState(incoming);
  if (previousIncoming !== incoming) {
    setPreviousIncoming(incoming);
    setActive(incoming.split("/")[0] as DrawerSearchTab);
    setSection(incoming);
  }
  const focusedTab = useRef<string | undefined>(undefined);
  useEffect(() => {
    setQuery("");
    const root = incoming.split("/")[0];
    const changedTab = focusedTab.current !== root;
    focusedTab.current = root;
    if (!changedTab || incoming.includes("/") || (connectionId && incoming === "channels")) return;
    const focusPanel = () =>
      panels.current
        ?.querySelector<HTMLElement>("[data-panel]:not([hidden]) h2")
        ?.focus({ preventScroll: true });
    focusPanel();
    const frame = requestAnimationFrame(focusPanel);
    return () => cancelAnimationFrame(frame);
  }, [incoming, setQuery, connectionId]);
  useEffect(() => {
    if (connectionId && selected === "channels") return;
    panels.current
      ?.querySelector<HTMLElement>("[data-panel]:not([hidden]) [data-panel-scroll]")
      ?.scrollTo?.({ top: 0 });
  }, [selected, connectionId]);
  const availability = agentDetailsAvailability(agent, {
    orgChannels,
    hasOrgChannel,
    voice: voiceEnabled,
    learning: learningEnabled,
    owner,
  });
  const page = agentDetailsSections.available(section, availability) ?? "overview";
  const [visited, setVisited] = useState(() => new Set(agentDetailsSections.ancestry(page)));
  if (!visited.has(page)) setVisited(new Set([...visited, ...agentDetailsSections.ancestry(page)]));
  const automationCache = useTriggers(undefined, false);
  const automationCount = automationCache.data && !automationCache.error ? automationCache.data.triggers.filter(row => row.delivery.type === "assistant" && row.delivery.agent_id === agent.id).length : undefined;
  const workspaceSettings = useNyxBotSettings(false);
  const workspaceMode = !workspaceSettings.error ? workspaceSettings.data?.access_mode : undefined;
  function navigateSection(next: AgentDetailsSection = "overview") {
    if (!router) {
      setSection(next);
      setActive(next.split("/")[0] as DrawerSearchTab);
    }
    if (router)
      void router.navigate({
        to: "/assistant",
        search: (previous) => ({
          ...previous,
          agent: agent.id,
          details: next,
          proposal: next === "skills/learning" ? previous.proposal : undefined,
        }),
        state: previous => previous,
        replace: true,
        resetScroll: false,
      });
  }

  const icons = {
    overview: UserRound,
    grants: ShieldCheck,
    skills: BookOpen,
    memory: Brain,
    channels: MessageSquare,
  };
  function rows(root: AgentDetailsTab) {
    return (
      <GroupedList>
        {AGENT_DETAILS_SECTIONS.filter(
          (child) => agentDetailsSections.parent(child) === root && availability[child] !== false,
        ).map((child) => {
          const Icon = child.endsWith("access")
            ? ShieldCheck
            : child.endsWith("voice")
              ? AudioLines
              : child.endsWith("automations")
                ? CalendarClock
                : child.endsWith("machines")
                  ? Monitor
                  : child.endsWith("connections")
                    ? Plug
                    : child.endsWith("guests")
                      ? Users
                      : child.endsWith("operations")
                        ? Wrench
                        : child === "overview/chat"
                          ? MessageCircle
                          : child === "overview/ownership"
                            ? BadgeInfo
                            : child === "skills/learning" ? ListChecks : icons[root];
          const summary =
            child === "overview/persona"
              ? agentTitle(agent)
              : child === "overview/access"
                ? agent.access_mode_override
                  ? translateUi(ACCESS_MODE_LABEL[agent.access_mode_override])
                  : workspaceMode ? translateUi("Workspace setting ({{mode}})", { mode: translateUi(ACCESS_MODE_LABEL[workspaceMode]) }) : undefined
                : child === "overview/chat"
                  ? translateUi(agent.channel_checking_notice ? "On" : "Off")
                  : child === "overview/ownership"
                    ? agent.owner_kind === "org" || agent.share_role
                      ? (agent.owner_name ?? agent.owner_id)
                      : translateUi("Personal agent")
                    : child === "overview/automations" && automationCount !== undefined
                      ? String(automationCount)
                    : child === "grants/services" && agent.kind === "specialist"
                      ? String(agent.services.length)
                      : undefined;
          return (
            <GroupedDisclosure
              key={child}
              leading={<Icon aria-hidden />}
              accessory={
                summaries && summary ? (
                  <span className="text-13 text-text-tertiary">{summary}</span>
                ) : undefined
              }
              data-drawer-page-trigger={child}
              onClick={() => navigateSection(child)}
              search={{
                id: child,
                title: AGENT_DETAILS_TITLES[child],
                keywords: translateUi(AGENT_DETAILS_TITLES[child]),
              }}
            >
              {translateUi(AGENT_DETAILS_TITLES[child])}
            </GroupedDisclosure>
          );
        })}
      </GroupedList>
    );
  }
  function detailPage(
    value: AgentDetailsSection,
    renderGrants: (section: GrantsSection) => ReactNode,
    renderProfile: (section: ProfileSection) => ReactNode,
  ) {
    if (value in AGENT_PROFILE_FIELDS)
      return agent.share_role ? (
        <SharedAgentProfile agent={agent} />
      ) : (
        renderProfile(value as ProfileSection)
      );
    if (value === "overview/automations") return <AgentAutomations agentId={agent.id} />;
    if (value === "overview/ownership")
      return (
        <>
          <GroupedList
            footer={
              agent.owner_kind === "org"
                ? translateUi(
                    "Maintained by Admins and Members. Your role: {{value1}}.{{value2}}",
                    {
                      value1: translateUi(agent.org_role ?? "member"),
                      value2:
                        agent.can_maintain === false
                          ? translateUi(" Read-only; viewers cannot maintain or chat.")
                          : translateUi(
                              " Chats are private to you; memory is shared with the organization.",
                            ),
                    },
                  )
                : agent.share_role
                  ? translateUi("Your role: {{value1}}. Your chats and notes stay private.", {
                      value1: agent.share_role,
                    })
                  : undefined
            }
          >
            <GroupedRow
              title={
                agent.owner_kind === "org"
                  ? (agent.owner_name ?? agent.owner_id)
                  : agent.share_role
                    ? translateUi("Shared by {{value1}}", {
                        value1: agent.owner_name ?? agent.owner_id,
                      })
                    : translateUi("Personal agent")
              }
            />
          </GroupedList>
          <ShareAgentDialog agent={agent} />
          {!agent.share_role && agent.kind === "specialist" && agent.can_maintain !== false && (
            <Lifecycle agent={agent} onDeleted={onDeleted} />
          )}
        </>
      );
    if (value === "grants/services")
      return agent.share_role ? (
        <SharedConsentCards agentId={agent.id} />
      ) : agent.kind === "specialist" ? (
        renderGrants("services")
      ) : (
        <GroupedList
          footer={translateUi(
            "NyxBot runs with full access to your connected services and account, and is the only agent that can grant access to specialists. Machine capabilities are assigned separately. Its access mode decides when it asks you first.",
          )}
        >
          <GroupedRow
            search={{
              id: "nyxbot-full-access",
              title: "Connected services and account",
              breadcrumb: "Access",
              keywords: "Full access",
            }}
            title={translateUi("Connected services and account")}
            accessory={translateUi("Full access")}
          />
        </GroupedList>
      );
    if (value === "grants/machines")
      return (
        <>
          {agent.kind === "specialist" && renderGrants("machines")}
          {(agent.kind === "nyxbot" || agent.can_maintain !== false) && (
            <MachineCapabilities agentId={agent.id} disabled={readOnly} />
          )}
        </>
      );
    if (value === "grants/operations")
      return (
        <AgentOperationScopes
          agentId={agent.id}
          disabled={readOnly}
          drillIn
          visible={page === "grants/operations"}
          onClose={onClose}
          detailContainer={pageContainer}
        />
      );
    if (value === "grants/guests")
      return (
        <>
          {agent.owner_kind !== "org" && (
            <CollaborationGuestControls agentId={agent.id} disabled={readOnly} />
          )}
          {renderGrants("guests")}
          {canManageGuests && (
            <AgentOperationScopes
              agentId={agent.id}
              disabled={readOnly}
              guests
              drillIn
              visible={page === "grants/guests"}
              onClose={onClose}
              detailContainer={pageContainer}
            />
          )}
        </>
      );
    if (value === "grants/connections") return <AgentConnections agent={agent} />;
    if (value === "grants/voice") return <AgentVoiceService agent={agent} />;
    if (value === "skills/installed")
      return (
        <AgentSkills
          id={SKILLS_SECTION_ID}
          agentId={agent.id}
          readOnly={readOnly || Boolean(agent.share_role)}
        />
      );
    if (value === "skills/learning")
      return (
        <AgentLearningReview
          key={proposalId ?? ""}
          agentId={agent.id}
          proposalId={proposalId}
          readOnly={readOnly}
        />
      );
    return null;
  }
  return (
    <>
      {router && <AgentDetailsAddress agentId={agent.id} section={section} page={page} />}
      <ProfileEditor agent={agent} modelsEnabled={visited.has("overview/persona")}>
        {(renderProfile) => (
          <GrantsEditor agent={agent} requestMode={requestMode} visited={visited}>
            {(renderGrants) => (
              <DrawerSectionInfoContext value={true}>
                <DrawerSearchProvider
                  query={query}
                  fallbackEntries={AGENT_DETAILS_SECTIONS.filter(
                    value => availability[value] !== false && !visited.has(value),
                  ).flatMap(value =>
                    agentDetailsSearchTerms(value, agent, advancedControls).slice(1).map((title, index) => ({
                      id: `unvisited/${value}/${index}`,
                      title,
                      tab: value.split("/")[0] as AgentDetailsTab,
                      pages: agentDetailsSections.ancestry(value),
                      breadcrumb: translateUi(AGENT_DETAILS_TITLES[value]),
                      reveal: () => navigateSection(value),
                      target: () => pageContainer?.querySelector<HTMLElement>(`[data-grouped-section="${value}"] h2`) ?? null,
                    })),
                  )}
                >
                  <GroupedFormScope viewport>
                    <FormViewport className="flex min-h-0 flex-1 flex-col">
                      <SegmentedControl
                        value={selected}
                        onValueChange={(value) => navigateSection(value as AgentDetailsSection)}
                        className="flex min-h-0 flex-1 flex-col gap-0"
                      >
                        <AgentDrawerSearch
                          host={searchHost}
                          open={searchOpen}
                          setOpen={setSearchOpen}
                          title={agentTitle(agent)}
                          query={query}
                          setQuery={setQuery}
                          selected={selected}
                          selectTab={(value) => navigateSection(value)}
                          container={detailContainer}
                        />
                        <div
                          className={`relative order-1 flex min-h-0 flex-col gap-2 ${searching ? "shrink-0" : "flex-1"}`}
                        >
                          <CompactChrome className="px-4">
                            <SegmentedControlList
                              aria-label={translateUi("Agent details sections")}
                              className="gap-0.5 p-0.5"
                            >
                              {tabs.map(([value, label]) => (
                                <SegmentedControlTrigger
                                  key={value}
                                  value={value}
                                  className="min-h-6 px-1.5 py-1 text-14 [[data-compact=true]_&]:py-0.5"
                                >
                                  {translateUi(label === "Memory" ? "Agent memory" : label)}
                                </SegmentedControlTrigger>
                              ))}
                            </SegmentedControlList>
                          </CompactChrome>
                          <div
                            ref={setPanels}
                            hidden={searching}
                            style={searching ? { display: "none" } : undefined}
                            className="relative min-h-0 flex-1 overflow-hidden px-4"
                          >
                            {tabs.map(([value, label]) => (
                              <SegmentedControlContent
                                key={value}
                                forceMount
                                value={value}
                                hidden={selected !== value}
                                data-panel={value}
                                className="h-full min-h-0 flex-col space-y-0 data-[state=active]:flex data-[state=inactive]:hidden"
                              >
                                <h2 tabIndex={-1} className="sr-only outline-none">
                                  {translateUi(label === "Memory" ? "Agent memory" : label)}
                                </h2>
                                <DrawerSearchTabContext.Provider value={value}>
                                  <GroupedDetailContext.Provider
                                    value={{
                                      container: pageContainer,
                                      visible: selected === value && !searching,
                                    }}
                                  >
                                    <GroupedFormScope collect>
                                      <CompactScroll
                                        data-panel-scroll=""
                                        active={selected === value && !searching}
                                        className="assistant-scrollbar flex-1 space-y-6 pb-6"
                                      >
                                        {value === "overview" ||
                                        value === "grants" ||
                                        value === "skills" ? (
                                          rows(value)
                                        ) : value === "memory" && visited.has(value) ? (
                                          agent.share_role ? (
                                            <SharedAgentMemory agent={agent} />
                                          ) : (
                                            <MemoryList agent={agent} memory={memory} />
                                          )
                                        ) : value === "channels" && visited.has(value) ? (
                                          <ChannelBotsManager
                                            agents={agents}
                                            agent={agent}
                                            connectionId={
                                              selected === "channels" ? connectionId : undefined
                                            }
                                          />
                                        ) : null}
                                      </CompactScroll>
                                      <SectionPages
                                        sections={AGENT_DETAILS_SECTIONS}
                                        titles={AGENT_DETAILS_TITLES}
                                        root={value}
                                        rootTitle={AGENT_DETAILS_TITLES[value]}
                                        selected={page}
                                        navigate={navigateSection}
                                        available={availability}
                                        render={(value) =>
                                          detailPage(value, renderGrants, renderProfile)
                                        }
                                      />
                                      <GroupedFormBar />
                                    </GroupedFormScope>
                                  </GroupedDetailContext.Provider>
                                </DrawerSearchTabContext.Provider>
                              </SegmentedControlContent>
                            ))}
                          </div>
                        </div>
                      </SegmentedControl>
                    </FormViewport>
                    <GroupedFormBar viewport />
                  </GroupedFormScope>
                </DrawerSearchProvider>
              </DrawerSectionInfoContext>
            )}
          </GrantsEditor>
        )}
      </ProfileEditor>
    </>
  );
}

/** Canonicalize addresses only within a router; standalone sheets keep local navigation. */
function AgentDetailsAddress({ agentId, section, page }: { agentId: string; section: AgentDetailsSection; page: AgentDetailsSection }) {
  const router = useRouter();
  const { value: raw, settled } = useSectionSearchParam("details");
  useEffect(() => {
    if (settled && (section !== page || (raw !== undefined && raw !== page)))
      void router.navigate({ to: "/assistant", search: previous => ({ ...previous, agent: agentId, details: page }), state: previous => previous, replace: true, resetScroll: false });
  }, [settled, section, page, raw, router, agentId]);
  return null;
}

/** The access-mode select's value for "follow the workspace mode". */
const INHERIT = "inherit";

type ProfileSection = keyof typeof AGENT_PROFILE_FIELDS;

function ProfileEditor({ agent, modelsEnabled, children }: { readonly agent: AssistantAgent; readonly modelsEnabled: boolean; readonly children: (render: (section: ProfileSection) => ReactNode) => ReactNode }) {
  useUiTranslation();
  const advancedControls = useFeature(FEATURE_FLAG.ASSISTANT_MODEL_CONTROLS);
  const update = useUpdateNyxBotAgent();
  const models = useNyxAgentModels(advancedControls && modelsEnabled, { agentId: agent.id });
  const workspaceMode = useNyxBotSettings(false).data?.access_mode;
  const nyxbot = agent.kind === "nyxbot";
  const readOnly = agent.status === "destroyed" || agent.can_maintain === false;
  const form = useAppForm<AssistantAgentProfile>({
    resolver: zodResolver(assistantAgentProfileSchema(agent.kind)),
    resetOptions: { keepDirtyValues: true },
    values: {
      name: agent.name,
      display_name: agent.display_name ?? "",
      description: agent.description,
      persona: agent.persona ?? "",
      voice_instructions: agent.voice_instructions ?? "",
      model: agent.model ?? "",
      access_mode_override: agent.access_mode_override ?? null,
      channel_checking_notice: agent.channel_checking_notice ?? false,
    },
  });
  const [error, setError] = useState<string>();
  const { dirtyFields } = form.formState;

  async function save(values: AssistantAgentProfile) {
    setError(undefined);
    const dirty = dirtyFields;
    try {
      // Only what changed; an emptied display name or persona clears it.
      await update.mutateAsync({
        id: agent.id,
        ...(dirty.name && !nyxbot ? { name: values.name } : {}),
        ...(dirty.display_name ? { display_name: values.display_name } : {}),
        ...(dirty.description ? { description: values.description } : {}),
        ...(dirty.persona ? { persona: values.persona } : {}),
        ...(dirty.voice_instructions ? { voice_instructions: values.voice_instructions } : {}),
        ...(dirty.model ? { model: values.model } : {}),
        ...(dirty.access_mode_override
          ? { access_mode_override: values.access_mode_override }
          : {}),
        ...(dirty.channel_checking_notice
          ? { channel_checking_notice: values.channel_checking_notice }
          : {}),
      });
      form.reset(values, { keepDirtyValues: false });
    } catch (cause) {
      setError(errorMessage(cause, "Could not save. Try again."));
    }
  }

  const fields = {
    name: (
      <FormField
        control={form.control}
        name="name"
        render={({ field }) => (
          <GroupedRow
            search={{
              id: "profile-name",
              title: "Name",
              breadcrumb: "Overview › Profile",
              keywords: String(field.value ?? ""),
            }}
            asChild
            className="grouped-field"
          >
            <FormItem>
              <FormLabel>{translateUi(AGENT_PROFILE_FIELDS["overview/persona"].name)}</FormLabel>
              <FormControl>
                <GroupedInput
                  maxLength={32}
                  disabled={readOnly}
                  {...field}
                  onChange={(event) => field.onChange(event.target.value.toLowerCase())}
                />
              </FormControl>
              <FormDescription className="basis-full type-secondary text-text-tertiary">
                {translateUi("The @handle used in groups and by NyxBot.")}
              </FormDescription>
              <GroupedFooter>
                <FormMessage />
              </GroupedFooter>
            </FormItem>
          </GroupedRow>
        )}
      />
    ),
    display_name: (
      <FormField
        control={form.control}
        name="display_name"
        render={({ field }) => (
          <GroupedRow
            search={{
              id: "profile-display_name",
              title: "Display name",
              breadcrumb: "Overview › Profile",
              keywords: String(field.value ?? ""),
            }}
            asChild
            className="grouped-field"
          >
            <FormItem>
              <FormLabel>
                {translateUi(AGENT_PROFILE_FIELDS["overview/persona"].display_name)}
              </FormLabel>
              <FormControl>
                <GroupedInput
                  maxLength={ASSISTANT_AGENT_DISPLAY_NAME_MAX}
                  disabled={readOnly}
                  autoComplete="off"
                  placeholder={nyxbot ? translateUi("NyxBot") : agent.name}
                  {...field}
                />
              </FormControl>
              <FormDescription className="basis-full type-secondary text-text-tertiary">
                {translateUi("Shown instead of ")}
                {nyxbot ? translateUi("NyxBot") : translateUi("@{{name}}", { name: agent.name })}
                {translateUi(". Leave empty to use it. ")}
              </FormDescription>
              <GroupedFooter>
                <FormMessage />
              </GroupedFooter>
            </FormItem>
          </GroupedRow>
        )}
      />
    ),
    model: (
      <FormField
        control={form.control}
        name="model"
        render={({ field }) => (
          <GroupedRow
            search={{
              id: "profile-model",
              title: "Model",
              breadcrumb: "Overview › Profile",
              keywords: `${field.value ?? ""} ${
                models.data
                  ?.filter((model) => model.id === field.value)
                  .map(modelDisplayName)
                  .join(" ") ?? ""
              }`,
            }}
            asChild
            className="grouped-field"
          >
            <FormItem>
              <FormLabel>{translateUi(AGENT_PROFILE_FIELDS["overview/persona"].model)}</FormLabel>
              <ModelSelect
                grouped
                label={translateUi("Agent model")}
                agentId={agent.id}
                value={field.value}
                disabled={readOnly}
                onChange={field.onChange}
              />
              <FormDescription className="basis-full type-secondary text-text-tertiary">
                {translateUi(
                  "Used by this agent's new threads. Each thread can switch from its composer.",
                )}
              </FormDescription>
              <GroupedFooter>
                <FormMessage />
              </GroupedFooter>
            </FormItem>
          </GroupedRow>
        )}
      />
    ),
    access_mode_override: (
      <CheckmarkList
        label={translateUi("Agent access mode")}
        value={form.watch("access_mode_override") ?? INHERIT}
        disabled={readOnly}
        footer={
          error ? (
            <p role="alert" className="text-destructive">
              {translateUi(error)}
            </p>
          ) : undefined
        }
        onChange={(value) =>
          form.setValue(
            "access_mode_override",
            value === INHERIT ? null : nyxAgentAccessModeSchema.parse(value),
          )
        }
        options={[
          {
            value: INHERIT,
            label: translateUi("Workspace setting"),
            description: workspaceMode ? translateUi(ACCESS_MODE_LABEL[workspaceMode]) : undefined,
          },
          ...nyxAgentAccessModeSchema.options.map((value) => ({
            value,
            label: translateUi(ACCESS_MODE_LABEL[value]),
            description: translateUi(ACCESS_MODE_DESCRIPTION[value]),
          })),
        ]}
      />
    ),
    channel_checking_notice: (
      <FormField
        control={form.control}
        name="channel_checking_notice"
        render={({ field }) => (
          <GroupedRow
            search={{
              id: "profile-channel_checking_notice",
              title: "Channel progress reply",
              breadcrumb: "Overview › Profile",
              keywords: field.value ? "On" : "Off",
            }}
            asChild
            className="grouped-field"
          >
            <FormItem>
              <div className="flex items-center justify-between gap-3">
                <FormLabel className="text-foreground">
                  {translateUi(AGENT_PROFILE_FIELDS["overview/chat"].channel_checking_notice)}
                </FormLabel>
                <FormControl>
                  <Switch
                    checked={field.value ?? false}
                    onCheckedChange={field.onChange}
                    disabled={readOnly}
                  />
                </FormControl>
              </div>
              <FormDescription className="basis-full type-secondary text-text-tertiary">
                {translateUi(
                  "After five seconds, send “Got it, let me check and get back to you.” if an external check is needed. Attached skill reads and tool discovery do not trigger it. The completed answer follows in the same chat.",
                )}
              </FormDescription>
            </FormItem>
          </GroupedRow>
        )}
      />
    ),
    description: (
      <FormField
        control={form.control}
        name="description"
        render={({ field }) => (
          <GroupedRow
            search={{
              id: "profile-description",
              title: nyxbot ? "Notes" : "Role",
              breadcrumb: "Overview › Profile",
              keywords: `Description ${field.value ?? ""}`,
            }}
            asChild
            className="grouped-field"
          >
            <FormItem>
              <FormLabel>
                {nyxbot
                  ? translateUi(AGENT_NOTES_LABEL)
                  : translateUi(AGENT_PROFILE_FIELDS["overview/persona"].description)}
              </FormLabel>
              <FormControl>
                <GroupedTextarea
                  className="min-h-20"
                  maxLength={2048}
                  disabled={readOnly}
                  placeholder={
                    nyxbot ? translateUi("How NyxBot should work with you (optional).") : undefined
                  }
                  {...field}
                />
              </FormControl>
              <GroupedFooter>
                <FormMessage />
              </GroupedFooter>
            </FormItem>
          </GroupedRow>
        )}
      />
    ),
    persona: (
      <FormField
        control={form.control}
        name="persona"
        render={({ field }) => (
          <GroupedRow
            search={{
              id: "profile-persona",
              title: "Persona",
              breadcrumb: "Overview › Profile",
              keywords: String(field.value ?? ""),
            }}
            asChild
            className="grouped-field"
          >
            <FormItem>
              <FormLabel>{translateUi(AGENT_PROFILE_FIELDS["overview/persona"].persona)}</FormLabel>
              <FormControl>
                <GroupedTextarea
                  className="min-h-20"
                  maxLength={ASSISTANT_AGENT_PERSONA_MAX}
                  disabled={readOnly}
                  placeholder={PERSONA_HINT}
                  {...field}
                />
              </FormControl>
              <FormDescription className="basis-full type-secondary text-text-tertiary">
                {translateUi("Personality and tone only; it never changes what the agent may do.")}
              </FormDescription>
              <GroupedFooter>
                <FormMessage />
              </GroupedFooter>
            </FormItem>
          </GroupedRow>
        )}
      />
    ),
    voice_instructions: (
      <FormField
        control={form.control}
        name="voice_instructions"
        render={({ field }) => (
          <GroupedRow
            search={{
              id: "profile-voice-instructions",
              title: "Voice instructions",
              breadcrumb: "Overview › Profile",
              keywords: String(field.value ?? ""),
            }}
            asChild
            className="grouped-field"
          >
            <FormItem>
              <FormLabel>
                {translateUi(AGENT_PROFILE_FIELDS["overview/voice"].voice_instructions)}
              </FormLabel>
              <FormControl>
                <GroupedTextarea
                  className="min-h-20"
                  maxLength={ASSISTANT_AGENT_VOICE_INSTRUCTIONS_MAX}
                  disabled={readOnly}
                  placeholder={translateUi(
                    "How to handle calls and use connected services (optional).",
                  )}
                  {...field}
                />
              </FormControl>
              <FormDescription className="basis-full type-secondary text-text-tertiary">
                {translateUi(
                  "Guidance for this Bot across all voice clients and connected services. Applies to new calls using its existing permissions.",
                )}
              </FormDescription>
              <GroupedFooter>
                <FormMessage />
              </GroupedFooter>
            </FormItem>
          </GroupedRow>
        )}
      />
    ),
  };
  function render(section: ProfileSection) {
    const keys = Object.keys(AGENT_PROFILE_FIELDS[section]) as (keyof AssistantAgentProfile)[];
    return (
      <section aria-label={translateUi("Profile")} className="space-y-3">
        <Form {...form}>
          <form
            aria-label={translateUi(AGENT_DETAILS_TITLES[section])}
            noValidate
            onSubmit={form.handleSubmit(save)}
            className="space-y-3"
          >
            {section !== "overview/access" && (
              <GroupedList
                title={undefined}
                footer={
                  error ? (
                    <p role="alert" className="text-destructive">
                      {translateUi(error)}
                    </p>
                  ) : undefined
                }
              >
                {Object.keys(AGENT_PROFILE_FIELDS[section])
                  .filter((name) => name !== "access_mode_override")
                  .map((name) => {
                    if ((name === "name" && nyxbot) || (name === "model" && !advancedControls))
                      return null;
                    return (
                      <span key={name} className="contents">
                        {fields[name as keyof typeof fields]}
                      </span>
                    );
                  })}
              </GroupedList>
            )}
            {section === "overview/access" && fields.access_mode_override}
            {readOnly ? null : (
              <GroupedFormActions
                changes={() =>
                  describeChanges(
                    form.formState.defaultValues,
                    form.getValues(),
                    (
                      [
                        { key: "name", label: "Name" },
                        { key: "display_name", label: "Display name" },
                        { key: "description", label: "Role / description" },
                        { key: "persona", label: "Persona" },
                        { key: "voice_instructions", label: "Voice instructions" },
                        { key: "channel_checking_notice", label: "Channel progress reply" },
                        {
                          key: "model",
                          label: "Model",
                          format: (value) =>
                            models.data?.find((model) => model.id === value)
                              ? modelDisplayName(models.data.find((model) => model.id === value)!)
                              : String(value || "Default model"),
                        },
                        {
                          key: "access_mode_override",
                          label: "Access mode",
                          format: labelValue(ACCESS_MODE_LABEL, "Workspace setting"),
                        },
                      ] satisfies ChangeField<keyof AssistantAgentProfile>[]
                    ).filter((field) => keys.includes(field.key)),
                  )
                }
                label={translateUi("profile")}
                dirty={keys.some((key) => Boolean(dirtyFields[key]))}
                pending={update.isPending}
                saveLabel={translateUi("Save")}
                save={form.handleSubmit(save)}
                discard={() => {
                  keys.forEach((key) => form.resetField(key));
                  setError(undefined);
                }}
              />
            )}
          </form>
        </Form>
      </section>
    );
  }
  return children(render);
}

type GrantsSection = "services" | "machines" | "guests";

/** One replacement payload and draft across the Access pages. */
function GrantsEditor({
  agent,
  requestMode,
  visited,
  children,
}: {
  readonly visited: ReadonlySet<AgentDetailsSection>;
  readonly agent: AssistantAgent;
  readonly requestMode?: "denied" | "routed_to_nyxbot";
  readonly children: (render: (section: GrantsSection) => ReactNode) => ReactNode;
}) {
  useUiTranslation();
  const orgChannels = useFeature(FEATURE_FLAG.ORG_SPECIALIST_CHANNELS);
  const canManageGuests =
    agent.owner_kind !== "org" || (orgChannels && agent.org_role === "admin");
  const readiness = useGuestReadiness(agent.id, visited.has("grants/guests") && agent.owner_kind !== "org" && !agent.share_role && agent.can_maintain !== false);
  const grants = useSetNyxBotAgentGrants();
  const inventory = useKeys({ enabled: visited.has("grants/services") || visited.has("grants/guests") });
  const nodes = useNodes({ enabled: visited.has("grants/machines") });
  const logins = useSavedLogins("all", visited.has("grants/machines"));
  const serviceName = (id: string) => { const key = inventory.data?.find((key) => key.id === id || key.slug === id); return key?.name || key?.label || id; };
  const readOnly = agent.status === "destroyed" || agent.can_maintain === false;
  const form = useAppForm<AssistantAgentGrants>({
    resolver: zodResolver(assistantAgentGrantsSchema),
    resetOptions: { keepDirtyValues: true },
    values: {
      services: agent.services,
      machines: agent.machines ?? [],
      logins: agent.logins ?? [],
      account_read: agent.account_read,
      guest_access: agent.guest_access,
    },
  });
  const [error, setError] = useState<string>();
  const services = form.watch("services");
  const { dirtyFields } = form.formState;

  async function save(values: AssistantAgentGrants) {
    setError(undefined);
    // Send only the levels changed here, for services being granted: the
    // server keeps the others, and drops levels of services taken away.
    const changed = Object.fromEntries(
      values.services
        .map((slug) => [slug, values.guest_access[slug] ?? "use"] as const)
        .filter(
          ([slug, level]) => level !== (agent.guest_access[slug] ?? "use"),
        ),
    );
    try {
      await grants.mutateAsync({
        id: agent.id,
        services: values.services,
        ...(dirtyFields.machines ? { machines: values.machines } : {}),
        ...(dirtyFields.logins ? { logins: values.logins } : {}),
        account_read: values.account_read,
        ...(Object.keys(changed).length ? { guest_access: changed } : {}),
      });
      form.reset(values, { keepDirtyValues: false });
    } catch (cause) {
      setError(errorMessage(cause, "Could not save the grants. Try again."));
    }
  }

  function render(section: GrantsSection) {
    const keys: (keyof AssistantAgentGrants)[] = section === "services" ? ["services", "account_read"] : section === "machines" ? ["machines", "logins"] : ["guest_access"];
    return (
    <section id={`${GRANTS_SECTION_ID}-${section}`} aria-label={translateUi("Grants")} className="space-y-3">
      <Form {...form}>
        <form
          aria-label={translateUi(section === "services" ? "Grants" : section === "machines" ? "Machine assignments" : "Guest access")}
          noValidate
          onSubmit={form.handleSubmit(save)}
          className="space-y-3"
        >
          {section === "services" && <FormField
            control={form.control}
            name="services"
            render={({ field }) => (
              <FormItem>
                <ServiceGrantPicker
                  footer={
                    <>
                      <p>{translateUi("Saving replaces the current grants on every thread.")}</p>
                      <section aria-label={translateUi("Access authority")}>
                        {accessAuthorityCopy(agent, requestMode)}
                      </section>
                      {error ? (
                        <p role="alert" className="text-destructive">
                          {translateUi(error)}
                        </p>
                      ) : null}
                    </>
                  }
                  org={agent.owner_kind === "org" ? agent.owner_id : undefined}
                  value={field.value}
                  onChange={field.onChange}
                  disabled={readOnly}
                />
              </FormItem>
            )}
          />}
          {section === "machines" && (
            ["machines", ...(agent.owner_kind === "org" ? [] : ["logins"])] as (
              | "machines"
              | "logins"
            )[]
          ).map((kind) => (
            <FormField
              key={kind}
              control={form.control}
              name={kind}
              render={({ field }) => (
                <FormItem>
                  <MachineGrantPicker
                    org={
                      agent.owner_kind === "org" ? agent.owner_id : undefined
                    }
                    kind={kind}
                    value={field.value ?? []}
                    onChange={field.onChange}
                    disabled={readOnly}
                  />
                </FormItem>
              )}
            />
          ))}
          {section === "guests" && services.length && canManageGuests ? (
            <FormField
              control={form.control}
              name="guest_access"
              render={({ field }) => (
                <FormItem>
                  <GuestAccessList
                    footer={<GuestReadiness readiness={readiness.data} />}
                    services={services}
                    value={field.value}
                    onChange={field.onChange}
                    disabled={readOnly}
                  />
                </FormItem>
              )}
            />
          ) : null}
          {section === "services" && agent.owner_kind !== "org" ? (
            <FormField
              control={form.control}
              name="account_read"
              render={({ field }) => (
                <FormItem>
                  <GroupedList
                    title={translateUi("Account")}
                    footer={
                      <>
                        {!services.length ? (
                          <GuestReadiness readiness={readiness.data} />
                        ) : null}
                        <p>{translateUi("Look up keys, services and nodes; never change them.")}</p>
                      </>
                    }
                  >
                    <GroupedRow search={{ id: "account-read", title: "Read my account", breadcrumb: "Access › Account", keywords: field.value ? "On" : "Off" }}>
                      <div className="mr-auto">
                        <FormLabel className="text-foreground">{translateUi("Read my account")}</FormLabel>
                      </div>
                      <FormControl>
                        <Switch
                          checked={field.value}
                          disabled={readOnly}
                          onCheckedChange={field.onChange}
                        />
                      </FormControl>
                    </GroupedRow>
                  </GroupedList>
                </FormItem>
              )}
            />
          ) : null}
          {readOnly ? null : (
            <GroupedFormActions
          changes={() => [...describeChanges(form.formState.defaultValues, form.getValues(), ([{ key: "services", label: "Services", list: (value) => changeList(value).map(serviceName) }, { key: "machines", label: "Machines", list: (value) => changeList(value).map((id) => nodes.data?.find((node) => node.id === id)?.name ?? id) }, { key: "logins", label: "Saved logins", list: (value) => changeList(value).map((id) => logins.data?.find((login) => login.id === id)?.label ?? id) }, { key: "account_read", label: "Read my account" }] satisfies ChangeField<keyof AssistantAgentGrants>[]).filter(field => keys.includes(field.key))), ...(section === "guests" ? recordChanges(form.formState.defaultValues?.guest_access, form.getValues("guest_access"), (key) => `Guest access · ${serviceName(key)}`, labelValue(GUEST_ACCESS_LABEL, "Use but not change or delete")) : [])]}
              label={translateUi("grants")}
              dirty={keys.some(key => Boolean(dirtyFields[key]))}
              pending={grants.isPending}
              saveLabel={translateUi("Save grants")}
              save={form.handleSubmit(save)}
              discard={() => {
                keys.forEach(key => form.resetField(key));
                setError(undefined);
              }}
            />
          )}
        </form>
      </Form>
      {section === "guests" && agent.owner_kind === "org" && canManageGuests && !readOnly ? (
        <OrgGuestGrantsAdoption agentId={agent.id} disabled={form.formState.isDirty || grants.isPending} />
      ) : null}
    </section>
  );
  }
  return children(render);
}

const GUEST_ACCESS_LABEL: Record<AssistantGuestAccess, string> = {
  get read() { return translateUi("Look things up only"); },
  get use() { return translateUi("Use, but not change or delete"); },
  get all() { return translateUi("Everything this agent can do"); },
};

/**
 * What people other than you may do with each granted service when they talk
 * to this agent in a group or shared chat. Anything behind your approval
 * stays yours at every level.
 */
function GuestAccessList({
  services,
  value,
  onChange,
  disabled,
  footer,
}: {
  readonly services: readonly string[];
  readonly value: Record<string, AssistantGuestAccess>;
  readonly onChange: (value: Record<string, AssistantGuestAccess>) => void;
  readonly disabled?: boolean;
  readonly footer?: React.ReactNode;
}) {
  useUiTranslation();
  return (
    <GroupedList
      title={translateUi("What others in its chats may do")}
      footer={
        <>{translateUi("For people other than you in the agent’s group and shared chats. “Use” lets them look things up, create and act (send a message, turn a light on), but not change or delete what already exists. Anything behind your approval stays yours. You can also ask NyxBot to change this.")}{footer}
        </>
      }
    >
      <div role="list" aria-label={translateUi("Guest access")}>
        {services.map((slug) => {
          const id = `guest-access-${slug}`;
          return (
            <GroupedRow key={slug} role="listitem" search={{ id: `guest-${slug}`, title: slug, breadcrumb: "Access › Guest access", keywords: GUEST_ACCESS_LABEL[value[slug] ?? "use"] }}>
              <Label
                htmlFor={id}
                className="min-w-0 flex-1 break-words text-foreground"
              >
                {slug}
              </Label>
              <Select
                value={value[slug] ?? "use"}
                disabled={disabled}
                onValueChange={(level) =>
                  onChange({ ...value, [slug]: level as AssistantGuestAccess })
                }
              >
                <GroupedSelectTrigger id={id} className="max-w-full">
                  <SelectValue />
                </GroupedSelectTrigger>
                <SelectContent>
                  {(
                    Object.keys(GUEST_ACCESS_LABEL) as AssistantGuestAccess[]
                  ).map((level) => (
                    <SelectItem key={level} value={level}>
                      {GUEST_ACCESS_LABEL[level]}
                    </SelectItem>
                  ))}
                </SelectContent>
              </Select>
            </GroupedRow>
          );
        })}
      </div>
    </GroupedList>
  );
}

function MemoryList({
  agent,
  memory,
}: {
  readonly agent: AssistantAgent;
  readonly memory: readonly AssistantAgentMemoryNote[];
}) {
  useUiTranslation();
  const forget = useForgetNyxBotMemory();
  const [error, setError] = useState<string>();
  const name = agentTitle(agent);
  const remember = useRememberNyxBotMemory();
  const memoryForm = useAppForm<{ text: string }>({
    resolver: zodResolver(assistantMemoryNoteSchema),
    defaultValues: { text: "" },
  });
  const [editing, setEditing] = useState<string>();
  async function saveNote({ text }: { text: string }) {
    setError(undefined);
    try {
      await remember.mutateAsync({
        agentId: agent.id,
        text,
        replaceId: editing,
      });
      memoryForm.reset({ text: "" });
      setEditing(undefined);
    } catch (cause) {
      setError(errorMessage(cause, "Could not save this note. Try again."));
    }
  }

  async function remove(note: AssistantAgentMemoryNote) {
    setError(undefined);
    try {
      await forget.mutateAsync({ agentId: agent.id, noteId: note.id });
    } catch (cause) {
      setError(errorMessage(cause, "Could not forget this note. Try again."));
    }
  }

  return (
    <section aria-label={translateUi("Agent memory")} className="space-y-6">
      {agent.status !== "destroyed" ? (
        <form
          aria-label={
            agent.owner_kind === "org" ? translateUi("Shared memory") : translateUi("Memory note")
          }
          onSubmit={memoryForm.handleSubmit(saveNote)}
          className="space-y-2"
        >
          <GroupedList
            title={editing ? translateUi("Edit note") : translateUi("Add a note")}
            footer={
              <>
                <p>
                  {agent.owner_kind === "org"
                    ? translateUi("Shared organization memory. Never save private member content here.")
                    : translateUi("What {{value1}} remembers across its threads and chat apps.", { value1: name })}
                </p>
                {error ? (
                  <p role="alert" className="text-destructive">
                    {translateUi(error)}
                  </p>
                ) : null}
                {memoryForm.formState.errors.text ? (
                  <p
                    id="memory-error"
                    role="alert"
                    className="text-destructive"
                  >
                    {memoryForm.formState.errors.text.message}
                  </p>
                ) : null}
              </>
            }
          >
            <GroupedRow search={{ id: "memory-note", title: editing ? "Edit note" : "Add a note", breadcrumb: "Memory", keywords: memoryForm.watch("text") }} className="grouped-field">
              <Label
                htmlFor="shared-memory-note"
              >
                {editing
                  ? translateUi("Edit shared note")
                  : agent.owner_kind === "org"
                    ? translateUi("Add shared note")
                    : translateUi("Note")}
              </Label>
              <GroupedTextarea
                id="shared-memory-note"
                className="min-h-20"
                maxLength={ASSISTANT_MEMORY_NOTE_MAX}
                aria-invalid={!!memoryForm.formState.errors.text}
                aria-describedby={
                  memoryForm.formState.errors.text ? "memory-error" : undefined
                }
                {...memoryForm.register("text")}
              />
            </GroupedRow>
          </GroupedList>
          <GroupedFormActions
            changes={() => describeChanges(memoryForm.formState.defaultValues, memoryForm.getValues(), [{ key: "text", label: "Note" }])}
            label={translateUi("memory note")}
            dirty={memoryForm.formState.isDirty}
            pending={remember.isPending}
            disabled={!memoryForm.watch("text").trim()}
            saveLabel={
              agent.owner_kind === "org" ? translateUi("Save shared note") : translateUi("Save note")
            }
            save={memoryForm.handleSubmit(saveNote)}
            discard={() => {
              memoryForm.reset({ text: "" });
              setEditing(undefined);
              setError(undefined);
            }}
          />
        </form>
      ) : null}
      {memory.length ? (
        <GroupedList
          title={translateUi("Notes")}
          footer={
            agent.status === "destroyed" ? (
              <>
                <p>
                  {agent.owner_kind === "org"
                    ? translateUi("Shared organization memory. Never save private member content here.")
                    : translateUi("What {{value1}} remembers across its threads and chat apps.", { value1: name })}
                </p>
                {error ? (
                  <p role="alert" className="text-destructive">
                    {translateUi(error)}
                  </p>
                ) : null}
              </>
            ) : undefined
          }
        >
          <div role="list" aria-label={translateUi("{{value1}} memory", { value1: name })}>
            {memory.map((note) => (
              <GroupedRow search={{ id: `memory-${note.id}`, title: note.text, breadcrumb: "Memory" }} key={note.id} role="listitem" className="items-start">
                <div className="min-w-0 flex-1 space-y-0.5">
                  <p className="whitespace-pre-wrap break-words text-14 text-foreground">
                    {note.text}
                  </p>
                  <p className="font-mono type-secondary text-text-tertiary">
                    {formatDateTime(note.updated_at)}
                  </p>
                </div>
                {agent.owner_kind === "org" && agent.status !== "destroyed" ? (
                  <Button
                    size="sm"
                    variant="ghost"
                    onClick={() => {
                      setEditing(note.id);
                      memoryForm.reset({ text: note.text });
                    }}
                  >{translateUi("Edit note")}</Button>
                ) : null}
                <Button
                  size="icon"
                  variant="ghost"
                  className="shrink-0"
                  aria-label={translateUi("Forget: {{value1}}", { value1: note.text.slice(0, 60) })}
                  disabled={forget.isPending || agent.status === "destroyed"}
                  onClick={() => void remove(note)}
                >
                  <Trash2 />
                </Button>
              </GroupedRow>
            ))}
          </div>
        </GroupedList>
      ) : (
        <GroupedList
          title={translateUi("Notes")}
          footer={
            agent.status === "destroyed"
              ? agent.owner_kind === "org"
                ? translateUi("Shared organization memory. Never save private member content here.")
                : translateUi("What {{value1}} remembers across its threads and chat apps.", { value1: name })
              : undefined
          }
        >
          <GroupedRow title={translateUi("Nothing remembered yet.")} />
        </GroupedList>
      )}
    </section>
  );
}

function Lifecycle({
  agent,
  onDeleted,
}: {
  readonly agent: AssistantAgent;
  readonly onDeleted: () => void;
}) {
  useUiTranslation();
  const userId = useAuthStore((state) => state.user?.id);
  const queryClient = useQueryClient();
  const destroy = useDestroyNyxBotAgent();
  const remove = useDeleteNyxBotAgent();
  const [confirm, setConfirm] = useState<"destroy" | "delete">();
  const [error, setError] = useState<string>();
  const destroyed = agent.status === "destroyed";
  const pending = destroy.isPending || remove.isPending;

  async function run() {
    setError(undefined);
    try {
      if (confirm === "destroy") {
        await destroy.mutateAsync(agent.id);
        // Its open threads become read-only.
        await queryClient.invalidateQueries({ queryKey: nyxBotQueryKeys.root(userId) });
      } else {
        await remove.mutateAsync(agent.id);
        nyxAgentTransport.forgetAgent(agent.id);
        onDeleted();
      }
      setConfirm(undefined);
    } catch (cause) {
      setError(errorMessage(cause, "That did not work. Try again."));
    }
  }

  return (
    <GroupedList
      title={translateUi("Danger zone")}
      aria-label={translateUi("Danger zone")}
      footer={
        destroyed
          ? translateUi("{{value1}} was destroyed{{value2}}. Deleting removes it and all of its threads for good.", { value1: agentTitle(agent), value2: agent.destroyed_at ? ` on ${formatDateTime(agent.destroyed_at)}` : "" })
          : translateUi("Stops its work, revokes its access and disconnects its channel bots. Its threads stay readable.")
      }
    >
      <GroupedRow search={{ id: "lifecycle", title: destroyed ? "Delete permanently" : "Destroy", breadcrumb: "Overview" }} asChild interactive className="text-destructive">
        <button
          type="button"
          onClick={() => {
            setError(undefined);
            setConfirm(destroyed ? "delete" : "destroy");
          }}
        >
          {destroyed ? translateUi("Delete permanently") : translateUi("Destroy")}
        </button>
      </GroupedRow>
      <Dialog
        open={confirm !== undefined}
        onOpenChange={(open) => {
          if (!open && !pending) setConfirm(undefined);
        }}
      >
        <DialogContent className="md:max-w-md">
          <DialogHeader>
            <DialogTitle>
              {confirm === "delete"
                ? translateUi("Delete {{value1}} permanently?", { value1: agentTitle(agent) })
                : translateUi("Destroy {{value1}}?", { value1: agentTitle(agent) })}
            </DialogTitle>
            <DialogDescription>
              {confirm === "delete"
                ? translateUi("The agent, its memory and every thread with it are deleted. This cannot be undone.")
                : translateUi("Any running turn stops, its access is revoked and its channel bots are disconnected. Its threads stay here, read-only. This cannot be undone.")}
            </DialogDescription>
          </DialogHeader>
          {error ? (
            <p role="alert" className="text-14 text-destructive">
              {translateUi(error)}
            </p>
          ) : null}
          <DialogFooter>
            <Button
              variant="ghost"
              disabled={pending}
              onClick={() => setConfirm(undefined)}
            >{translateUi("Cancel")}</Button>
            <Button
              variant="destructive"
              isLoading={pending}
              onClick={() => void run()}
            >
              {confirm === "delete" ? translateUi("Delete permanently") : translateUi("Destroy")}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </GroupedList>
  );
}
