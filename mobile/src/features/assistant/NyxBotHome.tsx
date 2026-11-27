import { useContext } from "react";
import { WorkspaceCornerSensorContext } from "../../components/workspace/WorkspaceLayoutContext";
import {
  WORKSPACE_TITLE_MAX_FONT_SCALE,
  WORKSPACE_TITLE_MAX_LINES,
} from "../../components/workspace/workspaceHeaderMetrics";
import { useAppLanguage } from "../../i18n/LanguageProvider";
import { translateApp } from "../../i18n/appTranslation";
import { retryAgentRead, agentReadRetryDelay, } from "../../lib/api/agentReadRetry";
import { AgentAvatar, AgentAvatarStack } from "../../components/AgentAvatar";
import { useRef, useState, type PropsWithChildren } from "react";
import {
  Text,
  Pressable,
  ScrollView,
  StyleSheet,
  View,
  useWindowDimensions,
  type ScrollViewProps,
} from "react-native";
import { ChevronRight, Plus } from "lucide-react-native";
import { useQuery } from "@tanstack/react-query";
import { Label, Notice } from "../../components/workspace/WorkspaceUI";
import { SectionBadge } from "../../components/SectionBadge";
import { StatusBadge } from "../../components/StatusBadge";
import { agentsApi } from "../../lib/api/agentsApi";
// TODO(onboarding): disabled on mobile by user decision 2026-11-06; restore together.
// import { onboardingApi } from "../../lib/api/onboardingApi";
// import { FirstRunIntro } from "../onboarding/FirstRunIntro";
// import { NextStepList } from "../onboarding/NextStepList";
// import { onboardingKey } from "../onboarding/onboardingContracts";
import type { NextStep } from "../onboarding/onboardingContracts";
import type { AssistantAgent } from "../agents/agentContracts";
import { groupsApi } from "../../lib/api/groupsApi";
import { getCurrentUserProfileRequest } from "../../lib/api/http";
import { livePollInterval } from "../../lib/api/assistantLiveTransport";
import { formatRelativeTimeFromMs } from "../../lib/time";
import { useTheme } from "../../theme/ThemeContext";
import { useAgentAvatarsEnabled } from "../../lib/avatars/agentAvatarsFlag";
import { rosterActivityLabel, rosterAvatarMood } from "../agents/agentRosterMood";
import { fonts } from "../../theme/designTokens";
import { groupKeys, useGroupAvailability } from "../groups/useGroupRuntime";
import { useGroupsLive } from "../groups/useGroupsLive";
import { chatAgentLabel } from "./assistantAgentContext";
import {
  NYXBOT_PLACEHOLDER,
  ownNyxBot,
} from "../../lib/avatars/ownNyxBot";
import {
  AWAY_PREVIEW,
  agentOwnerSections,
  awayItems,
  greetingFor,
} from "./nyxBotHomeModel";
import { RIGHT_PANEL_WIDTH } from "../../components/workspace/workspaceLayout";
import { useRailScrollIndicator } from "../../components/workspace/workspaceFloatingInset";

type Props = {
  scrollProps?: ScrollViewProps;
  widePanel?: boolean;
  onChat: (
    agentId: string,
    conversationId?: string,
    acknowledgementId?: string,
  ) => void;
  onOpenCrew: (groupId: string) => void;
  onNewAgent: () => void;
  onNewCrew: () => void;
  /** Sends a next step's prompt to NyxBot. */
  onStartNextStep?: (step: NextStep) => void;
};

/** The empty primary NyxBot chat uses the same reads as the workspace lists. */
export function NyxBotHome({
  scrollProps,
  widePanel = false,
  onChat,
  onOpenCrew,
  onNewAgent,
  onNewCrew,
  onStartNextStep,
}: Props) {
  const cornerWindow = useContext(WorkspaceCornerSensorContext);
  const railIndicator = useRailScrollIndicator();
  const [hostWidth, setHostWidth] = useState<number>();
  useAppLanguage();
  const { colors } = useTheme();
  const { t } = useAppLanguage();
  const avatars = useAgentAvatarsEnabled();
  const { epoch, active, online, available } = useGroupAvailability();
  const live = useGroupsLive(epoch, active, available);
  const [now] = useState(() => new Date());
  const [shown, setShown] = useState(AWAY_PREVIEW);
  const agents = useQuery({
    queryKey: ["workspace", "agents", "list"],
    queryFn: ({ signal }) => agentsApi.list(signal),
    enabled: active,
    // Re-enabling a stale roster syncs immediately; polling only fills gaps.
    staleTime: 0,
    refetchInterval: active ? 30_000 : false,
    retry: (count, error) => available() && retryAgentRead(count, error),
    retryDelay: agentReadRetryDelay,
  });
  const crews = useQuery({
    queryKey: [...groupKeys(epoch), "list"],
    queryFn: ({ signal }) => groupsApi.list(signal),
    enabled: active,
    refetchInterval: livePollInterval(active ? 5000 : false, live),
    retry: false,
  });
  const profile = useQuery({
    queryKey: ["account", "profile"],
    queryFn: ({ signal }) => getCurrentUserProfileRequest(signal),
    enabled: active,
    staleTime: 60_000,
    retry: false,
  });
  // TODO(onboarding): disabled on mobile by user decision 2026-11-06; restore together.
  // const queryClient = useQueryClient();
  // const onboarding = useQuery({
  //   queryKey: onboardingKey,
  //   queryFn: ({ signal }) => onboardingApi.get(signal),
  //   enabled: active && Boolean(onStartNextStep),
  //   staleTime: 60_000,
  //   retry: false,
  // });
  // const hideSuggestions = useMutation({
  //   mutationFn: () => onboardingApi.hideSuggestions(),
  //   onSuccess: (next) => queryClient.setQueryData(onboardingKey, next),
  // });
  // const [intro, setIntro] = useState<"first_run" | "home">();
  // const introShown = useRef(false);
  // useEffect(() => {
  //   if (!onboarding.data?.show_intro || introShown.current) return;
  //   introShown.current = true;
  //   setIntro("first_run");
  // }, [onboarding.data?.show_intro]);
  // const nextSteps = onboarding.data?.suggestions_hidden
  //   ? []
  //   : (onboarding.data?.recommendations ?? []);
  const roster = agents.data?.agents ?? [];
  const nyxbot = ownNyxBot(roster);
  const sections = agentOwnerSections(roster).map((section) => ({
    ...section,
    rows: section.agents.map((agent) => {
      const mood = avatars ? rosterAvatarMood(agent) : undefined;
      const name = chatAgentLabel(agent);
      const status = t(rosterActivityLabel(mood ?? "idle"));
      return { agent, mood, name, status };
    }),
  }));
  const groups = crews.data ?? [];
  const latest = useRef({ roster, groups });
  latest.current = { roster, groups };
  const items = awayItems(roster);
  const firstName = profile.data?.display_name?.trim().split(/\s+/u)[0];
  const hasTeam =
    roster.some(
      (agent) => agent.kind === "specialist" && agent.status !== "destroyed",
    ) || groups.length > 0;
  const openAgent = (
    agentId: string,
    conversationId?: string,
    requestId?: string,
  ) => {
    const agent = latest.current.roster.find((agent) => agent.id === agentId);
    if (
      !available() ||
      !agent ||
      agent.status === "destroyed" ||
      agent.can_use === false
    )
      return;
    if (
      requestId &&
      !agent.pending_requests.some(
        (request) =>
          request.request_id === requestId &&
          request.conversation_id === conversationId,
      )
    )
      return;
    onChat(agentId, conversationId, requestId);
  };
  const content = (
    <>
      <View style={styles.greeting}>
        <AgentAvatar
          agent={nyxbot ?? NYXBOT_PLACEHOLDER}
          size="lg"
          mood={nyxbot && avatars ? rosterAvatarMood(nyxbot) : undefined}
        />
        <View style={styles.greetingText}>
          {/* A large title like every page's: capped growth so a word never
              breaks mid-word at the largest text size, two lines, then an
              ellipsis. */}
          <Label
            accessibilityRole="header"
            maxFontSizeMultiplier={
              cornerWindow ?WORKSPACE_TITLE_MAX_FONT_SCALE : undefined}
            numberOfLines={cornerWindow ?WORKSPACE_TITLE_MAX_LINES : undefined}
            ellipsizeMode="tail"
            style={styles.title}
          >
            {greetingFor(now, firstName)}
          </Label>
          <Label muted style={styles.lead}>
            {translateApp(
              "NyxBot is your personal agent. Ask it anything below, or check in on your team.",
            )}
          </Label>
        </View>
      </View>
      {!online ? (
        <Notice message={translateApp(
            "You're offline. Reconnect to refresh your agents and crews.",
          )}
        />
      ) : null}
      {/* TODO(onboarding): disabled on mobile by user decision 2026-11-06; restore together.
      {onStartNextStep && onboarding.isError ? (
        <View style={styles.group}>
          <Notice
            message="Your NyxBot intro could not load. Try again."
            tone="error"
          />
          <ActionButton
            label="Retry intro"
            variant="secondary"
            disabled={!online}
            onPress={() => void onboarding.refetch()}
          />
        </View>
      ) : null}
      {onStartNextStep && onboarding.data && !nextSteps.length ? (
        <ActionButton
          label="Replay intro"
          variant="secondary"
          onPress={() => setIntro("home")}
        />
      ) : null}
      {nextSteps.length && onStartNextStep ? (
        // Its own header, like the other home groups; the steps are cards already.
        <View style={styles.group}>
          <View style={styles.groupHeader}>
            <Label accessibilityRole="header" style={styles.sectionTitle}>
              Suggested for you
            </Label>
            <View style={styles.suggestionActions}>
              <ActionButton
                label="Replay intro"
                compact
                variant="secondary"
                onPress={() => setIntro("home")}
              />
              <ActionButton
                label="Hide"
                compact
                variant="secondary"
                disabled={!online || hideSuggestions.isPending}
                onPress={() => hideSuggestions.mutate()}
              />
            </View>
          </View>
          <NextStepList
            steps={nextSteps}
            disabled={!online}
            onStart={onStartNextStep}
          />
        </View>
      ) : null}
      {intro && onboarding.data && onStartNextStep ? (
        <FirstRunIntro
          visible
          state={onboarding.data}
          entryPoint={intro}
          firstName={firstName}
          online={online}
          onClose={() => setIntro(undefined)}
          onStart={onStartNextStep}
        />
      ) : null}
      */}
      {!hasTeam &&
      !agents.isPending &&
      !crews.isPending &&
      !agents.isError &&
      !crews.isError ? (
        <HomeGroup>
          <Quiet title={translateApp("Hi, I'm NyxBot.")}>
            {translateApp(
              "I'm here to help you use NyxID, manage your NyxID services and automate them.",
            )}
          </Quiet>
        </HomeGroup>
      ) : null}
      {!widePanel ? (
        <HomeGroup title={translateApp("While you were away")}>
        {agents.isError ? (
          <HomeAction
            label={translateApp(
                "Couldn’t refresh your agents. Tap to retry.",
              )}
              danger
            disabled={!active}
            onPress={() => {
              if (available()) void agents.refetch();
            }}
          />
        ) : null}
        {agents.isPending ? (
          <Quiet>{translateApp("Loading your agents…")}</Quiet>
        ) : items.length ? (
          [
            ...items.slice(0, shown).map((item) => {
              const name = chatAgentLabel(item.agent);
              const request =
                item.kind === "request" ? item.request : undefined;
              const failed = item.agent.last_reply?.status === "failed";
              const mood = avatars ? rosterAvatarMood(item.agent) : undefined;
              return (
                <HomeRow
                  hostWidth={hostWidth}
                  key={item.key}
                  leading={<AgentAvatar agent={item.agent} size="lg" mood={mood} />}
                  title={name}
                  badge={
                    request ? (
                      <SectionBadge label={translateApp("Needs a decision")}
                          tone="warning" />
                    ) : failed && item.agent.status !== "running" ? (
                      <StatusBadge label={translateApp("Failed")}
                          variant="decisionDenied" />
                    ) : (
                      <SectionBadge
                        label={
                          item.agent.status === "running"
                            ? translateApp("Running")
                              : translateApp("Replied")
                          }
                        tone={
                          item.agent.status === "running" ? "info" : "success"
                        }
                      />
                    )
                  }
                  detail={
                    item.kind === "request" ? item.request.summary : item.text
                  }
                  meta={
                    item.kind === "update" && item.at
                      ? formatRelativeTimeFromMs(Date.parse(item.at))
                      : undefined
                  }
                  accessibilityLabel={
                    request
                      ? translateApp(
                            "Review {{name}}'s request: {{summary}}",
                            { name: name, summary: request.summary },
                          )
                        : translateApp("Open {{name}}'s latest work", {
                            name: name,
                          })
                    }
                  disabled={!active || item.agent.can_use === false}
                  onPress={() =>
                    openAgent(
                      item.agent.id,
                      request?.conversation_id ??
                        item.agent.home_conversation_id ??
                        undefined,
                      request?.request_id,
                    )
                  }
                />
              );
            }),
            items.length > AWAY_PREVIEW ? (
              <HomeAction
                key="more"
                label={
                  shown < items.length
                    ? translateApp("Show More ({{count}})", {
                          count: items.length - shown,
                        })
                      : translateApp("Show Less")
                  }
                onPress={() =>
                  setShown((value) =>
                    value < items.length ? value + AWAY_PREVIEW : AWAY_PREVIEW,
                  )
                }
              />
            ) : null,
          ]
        ) : !agents.isError ? (
          <Quiet>
              {translateApp(
                "All caught up. Replies and requests from your specialists show up here.",
              )}
            </Quiet>
        ) : null}
      </HomeGroup>
      ) : null}
      {!widePanel
        ? sections.map((section, index) => (
        <HomeGroup
          key={section.id}
          title={index === 0 ? translateApp("Your agents") : undefined}
          subtitle={section.label}
          action={
            index === 0 ? (
              <HeaderAction
                label={translateApp("New agent")}
                    disabled={!active}
                onPress={() => {
                  if (available()) onNewAgent();
                }}
              />
            ) : undefined
          }
        >
          {section.rows.map(({ agent, mood, name, status }) => (
            <HomeRow
              hostWidth={hostWidth}
              key={agent.id}
              leading={<AgentAvatar agent={agent} size="lg" mood={mood} />}
              title={name}
              badge={
                agent.kind !== "nyxbot" ? (
                  agent.pending_acknowledgements > 0 ? (
                    <SectionBadge label={translateApp("Needs you")}
                          tone="warning" />
                  ) : (
                    <StatusBadge
                      label={agent.status === "running" ? translateApp("Running")
                              : translateApp("Idle")
                          }
                      variant={
                        agent.status === "running"
                          ? "modeChip"
                          : "decisionExpired"
                      }
                    />
                  )
                ) : undefined
              }
              detail={translateApp(
                    agent.kind === "nyxbot"
                  ? agent.can_use === false ? "Your personal agent · View only · {{description}}"
                        : "Your personal agent · {{description}}"
                      : agent.can_use === false
                        ? "@{{name}} · View only · {{description}}"
                        : "@{{name}} · {{description}}",
                    {
                      name: agent.name,
                      description: agent.description ||
                (agent.kind === "nyxbot"
                  ? translateApp("Full access to your services and account. Delegates to your specialists.",)
                        : translateApp("No role described yet.")),
                    },
                  )}
              accessibilityLabel={avatars
                ? t("Chat with {{name}}, {{status}}", { name, status })
                : translateApp("Chat with {{label}}", { label: chatAgentLabel(agent) })}
              disabled={!active || agent.can_use === false}
              onPress={() => openAgent(agent.id)}
            />
          ))}
        </HomeGroup>
      ))
        : null}
      {!widePanel && !roster.some((agent) => agent.status !== "destroyed") ? (
        <HomeGroup
          title={translateApp("Your agents")}
          action={
            <HeaderAction
              label={translateApp("New agent")}
              disabled={!active}
              onPress={() => {
                if (available()) onNewAgent();
              }}
            />
          }
        >
          <Quiet>
            {agents.isPending
              ? translateApp("Loading your agents…")
              : agents.isError
                ? translateApp("Your agents are unavailable. Try again above.")
                : translateApp("No agents yet.")}
          </Quiet>
        </HomeGroup>
      ) : null}
      <HomeGroup
        title={translateApp("Crews")}
        action={
          <HeaderAction
            label={translateApp("New crew")}
            disabled={!active}
            onPress={() => {
              if (available()) onNewCrew();
            }}
          />
        }
      >
        {crews.isError ? (
          <HomeAction
            label={translateApp("Couldn’t refresh your crews. Tap to retry.")}
            danger
            disabled={!active}
            onPress={() => {
              if (available()) void crews.refetch();
            }}
          />
        ) : null}
        {groups.map((group) => (
          <HomeRow
            hostWidth={hostWidth}
            key={group.id}
            leading={<AgentAvatarStack agents={group.members} max={3} />}
            title={group.name}
            badge={
              group.working_agent_ids.length ? (
                <SectionBadge
                  label={translateApp("{{count}} working", {
                    count: group.working_agent_ids.length,
                  })}
                  tone="info"
                />
              ) : undefined
            }
            detail={`${group.owner?.type === "org" ? `${group.owner.name} · ` : ""}${group.members.map(chatAgentLabel).join(", ")}`}
            meta={
              group.last_message_at
                ? formatRelativeTimeFromMs(Date.parse(group.last_message_at))
                : undefined
            }
            accessibilityLabel={translateApp("Open crew {{name}}", {
              name: group.name,
            })}
            disabled={!active}
            onPress={() => {
              if (
                available() &&
                latest.current.groups.some((row) => row.id === group.id)
              )
                onOpenCrew(group.id);
            }}
          />
        ))}
        {!groups.length ? (
          <Quiet>
            {crews.isPending
              ? translateApp("Loading crews…")
              : crews.isError
                ? translateApp("Your crews are unavailable. Try again above.")
                : translateApp(
                    "Put several agents in one chat. Mention one with @ to ask it directly; NyxBot answers the rest.",
                  )}
          </Quiet>
        ) : null}
      </HomeGroup>
    </>
  );
  return (
    <ScrollView
      keyboardShouldPersistTaps="handled"
      keyboardDismissMode="on-drag"
      {...scrollProps}
      style={[{ flex: 1, minHeight: 0 }, scrollProps?.style]}
      contentContainerStyle={[
        styles.content,
        scrollProps?.contentContainerStyle,
      ]}
      {...railIndicator}
    >
      <View
        style={styles.contentRows}
        onLayout={({ nativeEvent: { layout } }) => {
          if (layout.width > 0) setHostWidth(layout.width);
        }}
      >
        {content}
      </View>
    </ScrollView>
  );
}

/** Wide-window companion column; the home transcript omits these same groups. */
export function NyxBotHomePanel({
  onChat,
  onNewAgent,
}: {
  onChat: (
    agentId: string,
    conversationId?: string,
    requestId?: string,
  ) => void;
  onNewAgent: () => void;
}) {
  const cornerWindow = useContext(WorkspaceCornerSensorContext);
  const railIndicator = useRailScrollIndicator();
  const [hostWidth, setHostWidth] = useState(RIGHT_PANEL_WIDTH);
  useAppLanguage();
  const { active, available, online } = useGroupAvailability();
  const agents = useQuery({
    queryKey: ["workspace", "agents", "list"],
    queryFn: ({ signal }) => agentsApi.list(signal),
    enabled: active,
    staleTime: 0,
    refetchInterval: active ? 30_000 : false,
    retry: false,
  });
  const roster = agents.data?.agents ?? [];
  const settled = !!agents.data && !agents.isError && online;
  const updates = awayItems(roster).filter((item) => item.kind === "update");
  const requests = awayItems(roster).filter((item) => item.kind === "request");
  const open = (
    agent: AssistantAgent,
    conversationId?: string,
    requestId?: string,
  ) => {
    if (!available() || agent.can_use === false || agent.status === "destroyed")
      return;
    onChat(
      agent.id,
      conversationId ?? agent.home_conversation_id ?? undefined,
      requestId,
    );
  };
  const content = (
    <>
      <Label accessibilityRole="header" style={styles.panelTitle}>
        {translateApp("While you were away")}
      </Label>
      {!online ? (
        <Notice
          message={translateApp("You’re offline. Showing saved activity.")}
        />
      ) : null}
      {agents.isPending ? (
        <Quiet>{translateApp("Loading your agents…")}</Quiet>
      ) : null}
      {agents.isError ? (
        <HomeAction
          label={translateApp("Your agents are unavailable. Tap to retry.")}
          danger
          disabled={!active}
          onPress={() => {
            if (available()) void agents.refetch();
          }}
        />
      ) : null}
      <HomeGroup title={translateApp("Recent updates")}>
        {updates.slice(0, 3).map((item) => (
          <HomeRow
            hostWidth={hostWidth}
            key={item.key}
            leading={<AgentAvatar agent={item.agent} size="lg" />}
            title={chatAgentLabel(item.agent)}
            detail={item.kind === "update" ? item.text : undefined}
            meta={
              item.kind === "update" && item.at
                ? formatRelativeTimeFromMs(Date.parse(item.at))
                : undefined
            }
            accessibilityLabel={translateApp("Open {{name}}'s latest work", {
              name: chatAgentLabel(item.agent),
            })}
            disabled={!active || item.agent.can_use === false}
            onPress={() => open(item.agent)}
          />
        ))}
        {settled && !updates.length ? (
          <Quiet>{translateApp("All caught up.")}</Quiet>
        ) : null}
      </HomeGroup>
      <HomeGroup title={translateApp("Pending approvals")}>
        {requests.map((item) => (
          <HomeRow
            hostWidth={hostWidth}
            key={item.key}
            leading={<AgentAvatar agent={item.agent} size="lg" />}
            title={chatAgentLabel(item.agent)}
            badge={
              <SectionBadge
                label={translateApp("Needs a decision")}
                tone="warning"
              />
            }
            detail={item.kind === "request" ? item.request.summary : undefined}
            accessibilityLabel={translateApp("Review {{label}}'s request", {
              label: chatAgentLabel(item.agent),
            })}
            disabled={!active || item.agent.can_use === false}
            onPress={() =>
              item.kind === "request" &&
              open(
                item.agent,
                item.request.conversation_id,
                item.request.request_id,
              )
            }
          />
        ))}
        {settled && !requests.length ? (
          <Quiet>{translateApp("No pending approvals.")}</Quiet>
        ) : null}
      </HomeGroup>
      <HomeGroup
        title={translateApp("Your agents")}
        action={
          <HeaderAction
            label={translateApp("New agent")}
            disabled={!active}
            onPress={() => {
              if (available()) onNewAgent();
            }}
          />
        }
      >
        {roster
          .filter((agent) => agent.status !== "destroyed")
          .map((agent) => (
            <HomeRow
              hostWidth={hostWidth}
              key={agent.id}
              leading={<AgentAvatar agent={agent} size="lg" />}
              title={chatAgentLabel(agent)}
              detail={agent.description || agent.name}
              accessibilityLabel={translateApp("Chat with {{label}}", {
                label: chatAgentLabel(agent),
              })}
              disabled={!active || agent.can_use === false}
              onPress={() => open(agent)}
            />
          ))}
        {settled && !roster.some((agent) => agent.status !== "destroyed") ? (
          <Quiet>{translateApp("No agents yet.")}</Quiet>
        ) : null}
      </HomeGroup>
    </>
  );
  return (
    <ScrollView
      testID="nyxbot-home-right-panel"
      style={styles.panelScroll}
      contentContainerStyle={styles.panelContent}
      keyboardShouldPersistTaps="handled"
      {...railIndicator}
    >
      <View
        style={styles.panelRows}
        onLayout={({ nativeEvent: { layout } }) => {
          if (layout.width > 0) setHostWidth(layout.width);
        }}
      >
        {content}
      </View>
    </ScrollView>
  );
}

/**
 * An inset-grouped list on the home page: an optional section header with a
 * tinted action, then rows in one filled card that stands apart from the page.
 */
function HomeGroup({
  title,
  subtitle,
  action,
  children,
}: PropsWithChildren<{
  title?: string;
  subtitle?: string;
  action?: React.ReactNode;
}>) {
  useAppLanguage();
  const { colors } = useTheme();
  const rows = [children]
    .flat(Infinity as 1)
    .filter(
      (child): child is React.ReactElement =>
        !!child && typeof child === "object" && "props" in child,
    );
  return (
    <View style={styles.group}>
      {title || action ? (
        <View style={styles.groupHeader}>
          {title ? (
            <Label accessibilityRole="header" style={styles.sectionTitle}>
              {title}
            </Label>
          ) : (
            <View style={{ flex: 1 }} />
          )}
          {action}
        </View>
      ) : null}
      {subtitle ? (
        <Label muted style={styles.owner}>
          {subtitle}
        </Label>
      ) : null}
      <View style={[styles.card, { backgroundColor: colors.groupedFill }]}>
        {rows.map((row, index) => (
          <View key={row.key ?? index}>
            {index > 0 ? (
              <View
                style={[
                  styles.separator,
                  { backgroundColor: colors.borderSoft },
                ]}
              />
            ) : null}
            {row}
          </View>
        ))}
      </View>
    </View>
  );
}

/** A home list row: the whole row opens its chat or crew. */
function HomeRow({
  hostWidth,
  leading,
  title,
  badge,
  detail,
  meta,
  onPress,
  disabled,
  accessibilityLabel,
}: {
  hostWidth?: number;
  leading: React.ReactNode;
  title: string;
  badge?: React.ReactNode;
  detail?: string;
  meta?: string;
  onPress: () => void;
  disabled?: boolean;
  accessibilityLabel: string;
}) {
  useAppLanguage();
  const { colors } = useTheme();
  const { width, fontScale } = useWindowDimensions();
  const compact = (hostWidth ?? width) / Math.max(1, fontScale) < 360;
  const metadata = meta ? <Label muted style={styles.metadata}>{meta}</Label> : null;
  return (
    <Pressable
      accessibilityRole="button"
      accessibilityLabel={accessibilityLabel}
      accessibilityState={{ disabled: !!disabled }}
      disabled={disabled}
      onPress={onPress}
      style={({ pressed }) => [
        styles.row,
        pressed && !disabled && { backgroundColor: colors.navActive },
        disabled && { opacity: 0.55 },
      ]}
    >
      <View style={styles.leading}>{leading}</View>
      <View style={styles.rowBody}>
        <View style={[styles.rowTitle, compact && styles.stackedTitle]}>
          <Label numberOfLines={compact ? undefined : 1} style={styles.name}>
            {title}
          </Label>
          {badge}
        </View>
        {detail ? (
          <Label muted numberOfLines={compact ? undefined : 2} style={styles.body}>
            {detail}
          </Label>
        ) : null}
        {compact ? metadata : null}
      </View>
      {compact ? null : metadata}
      <ChevronRight size={18} color={colors.textMuted} />
    </Pressable>
  );
}

/** A tinted action row inside a group (retry, show more). */
function HomeAction({
  label,
  onPress,
  disabled,
  danger,
}: {
  label: string;
  onPress: () => void;
  disabled?: boolean;
  danger?: boolean;
}) {
  useAppLanguage();
  const { colors } = useTheme();
  return (
    <Pressable
      accessibilityRole="button"
      disabled={disabled}
      onPress={onPress}
      style={({ pressed }) => [
        styles.actionRow,
        pressed && !disabled && { backgroundColor: colors.navActive },
      ]}
    >
      <Label
        style={[
          styles.actionLabel,
          { color: danger ? colors.danger : colors.primary },
        ]}
      >
        {label}
      </Label>
    </Pressable>
  );
}

/** Section header action in tinted text, as iOS lists do. */
function HeaderAction({
  label,
  onPress,
  disabled,
}: {
  label: string;
  onPress: () => void;
  disabled?: boolean;
}) {
  useAppLanguage();
  const { colors } = useTheme();
  return (
    <Pressable
      accessibilityRole="button"
      accessibilityLabel={label}
      accessibilityState={{ disabled: !!disabled }}
      disabled={disabled}
      hitSlop={10}
      onPress={onPress}
      style={styles.headerAction}
    >
      <Plus size={16} color={disabled ? colors.textMuted : colors.primary} />
      <Label
        style={[
          styles.actionLabel,
          { color: disabled ? colors.textMuted : colors.primary },
        ]}
      >
        {label}
      </Label>
    </Pressable>
  );
}

function Quiet({ title, children }: PropsWithChildren<{ title?: string }>) {
  useAppLanguage();
  return (
    <View style={styles.quiet}>
      {title ? <Label style={styles.name}>{title}</Label> : null}
      <Label muted style={styles.body}>
        {children}
      </Label>
    </View>
  );
}

const styles = StyleSheet.create({
  panelScroll: { flex: 1, minWidth: 0 },
  panelContent: { padding: 16, paddingBottom: 32 },
  panelRows: { gap: 4 },
  panelTitle: {
    fontFamily: fonts.displaySemi,
    fontSize: 18,
    lineHeight: 24,
    paddingBottom: 4,
  },
  suggestionActions: { flexDirection: "row", gap: 8 },
  content: { paddingTop: 24, paddingBottom: 24 },
  contentRows: { gap: 28 },
  greeting: { flexDirection: "row", alignItems: "center", gap: 12 },
  greetingText: { flex: 1, gap: 6 },
  title: { fontFamily: fonts.displaySemi, fontSize: 24, lineHeight: 30 },
  lead: { fontSize: 14, lineHeight: 20 },
  group: { gap: 8 },
  groupHeader: {
    flexDirection: "row",
    alignItems: "center",
    gap: 8,
    paddingHorizontal: 4,
  },
  sectionTitle: {
    flex: 1,
    fontFamily: fonts.displaySemi,
    fontSize: 18,
    lineHeight: 24,
  },
  owner: {
    fontFamily: fonts.bodySemi,
    fontSize: 11,
    letterSpacing: 0.6,
    textTransform: "uppercase",
    paddingHorizontal: 16,
    paddingTop: 4,
  },
  card: { borderRadius: 14, overflow: "hidden" },
  separator: { height: StyleSheet.hairlineWidth, marginLeft: 64 },
  row: {
    flexDirection: "row",
    alignItems: "center",
    gap: 12,
    minHeight: 64,
    paddingVertical: 12,
    paddingHorizontal: 16,
  },
  leading: { width: 36, alignItems: "center" },
  rowBody: { flex: 1, minWidth: 0, gap: 3 },
  rowTitle: {
    flexDirection: "row",
    alignItems: "center",
    flexWrap: "wrap",
    gap: 8,
  },
  stackedTitle: { flexDirection: "column", alignItems: "flex-start" },
  name: {
    flexShrink: 1,
    fontFamily: fonts.bodySemi,
    fontSize: 15,
    lineHeight: 20,
  },
  body: { fontSize: 13, lineHeight: 18 },
  metadata: { fontSize: 12, lineHeight: 16 },
  quiet: { paddingVertical: 14, paddingHorizontal: 16, gap: 4 },
  actionRow: {
    minHeight: 48,
    justifyContent: "center",
    paddingHorizontal: 16,
  },
  actionLabel: { fontFamily: fonts.bodySemi, fontSize: 14, lineHeight: 20 },
  headerAction: {
    flexDirection: "row",
    alignItems: "center",
    gap: 4,
    minHeight: 44,
    paddingHorizontal: 4,
  },
});
