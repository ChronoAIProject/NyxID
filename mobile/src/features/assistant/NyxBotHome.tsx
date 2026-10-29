import { AgentAvatar, AgentAvatarStack } from "../../components/AgentAvatar";
import { useRef, useState, type PropsWithChildren } from "react";
import {
  ScrollView,
  StyleSheet,
  View,
  type ScrollViewProps,
} from "react-native";
import { useQuery } from "@tanstack/react-query";
import {
  ActionButton,
  Card,
  Label,
  Notice,
} from "../../components/workspace/WorkspaceUI";
import { SectionBadge } from "../../components/SectionBadge";
import { StatusBadge } from "../../components/StatusBadge";
import { agentsApi } from "../../lib/api/agentsApi";
import { groupsApi } from "../../lib/api/groupsApi";
import { getCurrentUserProfileRequest } from "../../lib/api/http";
import { livePollInterval } from "../../lib/api/assistantLiveTransport";
import { formatRelativeTimeFromMs } from "../../lib/time";
import { useTheme } from "../../theme/ThemeContext";
import { fonts } from "../../theme/designTokens";
import { groupKeys, useGroupAvailability } from "../groups/useGroupRuntime";
import { useGroupsLive } from "../groups/useGroupsLive";
import { chatAgentLabel } from "./assistantAgentContext";
import {
  AWAY_PREVIEW,
  agentOwnerSections,
  awayItems,
  greetingFor,
} from "./nyxBotHomeModel";

type Props = {
  scrollProps?: ScrollViewProps;
  onChat: (
    agentId: string,
    conversationId?: string,
    acknowledgementId?: string,
  ) => void;
  onOpenCrew: (groupId: string) => void;
  onNewAgent: () => void;
  onNewCrew: () => void;
};

/** The empty primary NyxBot chat uses the same reads as the workspace lists. */
export function NyxBotHome({
  scrollProps,
  onChat,
  onOpenCrew,
  onNewAgent,
  onNewCrew,
}: Props) {
  const { colors } = useTheme();
  const { epoch, active, online, available } = useGroupAvailability();
  const live = useGroupsLive(epoch, active, available);
  const [now] = useState(() => new Date());
  const [shown, setShown] = useState(AWAY_PREVIEW);
  const agents = useQuery({
    queryKey: ["workspace", "agents", "list"],
    queryFn: ({ signal }) => agentsApi.list(signal),
    enabled: active,
    refetchInterval: livePollInterval(active ? 5000 : false, live),
    retry: false,
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
  const roster = agents.data?.agents ?? [];
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
  return (
    <ScrollView
      keyboardShouldPersistTaps="handled"
      keyboardDismissMode="on-drag"
      {...scrollProps}
      contentContainerStyle={[
        styles.content,
        scrollProps?.contentContainerStyle,
      ]}
    >
      <View style={styles.greeting}>
        <AgentAvatar
          agent={{ id: "nyxbot", name: "NyxBot", kind: "nyxbot" }}
          size="lg"
        />
        <View style={styles.greetingText}>
          <Label accessibilityRole="header" style={styles.title}>
            {greetingFor(now)}
            {firstName ? `, ${firstName}` : ""}
          </Label>
          <Label muted style={styles.body}>
            NyxBot is your personal agent. Ask it anything below, or check in on
            your team.
          </Label>
        </View>
      </View>
      {!online ? (
        <Notice message="You're offline. Reconnect to refresh your agents and crews." />
      ) : null}
      {!hasTeam &&
      !agents.isPending &&
      !crews.isPending &&
      !agents.isError &&
      !crews.isError ? (
        <Card>
          <Label style={styles.sectionTitle}>Hi, I'm NyxBot.</Label>
          <Label muted style={styles.body}>
            I'm here to help you use NyxID, manage your NyxID services and
            automate them.
          </Label>
        </Card>
      ) : null}
      <Section title="While you were away">
        {agents.isError ? (
          <Notice
            tone="error"
            message="Couldn’t refresh your agents. Saved updates may be out of date."
          />
        ) : null}
        {agents.isError ? (
          <ActionButton
            label="Retry agents"
            compact
            variant="secondary"
            disabled={!active}
            onPress={() => {
              if (available()) void agents.refetch();
            }}
          />
        ) : null}
        {agents.isPending ? (
          <Quiet>Loading your agents...</Quiet>
        ) : items.length ? (
          <Card>
            {items.slice(0, shown).map((item) => {
              const name = chatAgentLabel(item.agent);
              const request =
                item.kind === "request" ? item.request : undefined;
              const failed = item.agent.last_reply?.status === "failed";
              return (
                <View
                  key={item.key}
                  style={[styles.listRow, { borderColor: colors.borderSoft }]}
                >
                  <View style={styles.rowTitle}>
                    <AgentAvatar agent={item.agent} size="lg" />
                    <Label style={styles.name}>{name}</Label>
                    {request ? (
                      <SectionBadge label="Needs a decision" tone="warning" />
                    ) : failed && item.agent.status !== "running" ? (
                      <StatusBadge label="Failed" variant="decisionDenied" />
                    ) : (
                      <SectionBadge
                        label={
                          item.agent.status === "running"
                            ? "Running"
                            : "Replied"
                        }
                        tone={
                          item.agent.status === "running" ? "info" : "success"
                        }
                      />
                    )}
                  </View>
                  <Label muted numberOfLines={2} style={styles.body}>
                    {item.kind === "request" ? item.request.summary : item.text}
                  </Label>
                  <View style={styles.actions}>
                    {item.kind === "update" && item.at ? (
                      <Label muted style={styles.metadata}>
                        {formatRelativeTimeFromMs(Date.parse(item.at))}
                      </Label>
                    ) : null}
                    <ActionButton
                      label={request ? "Review" : "Open"}
                      compact
                      variant="secondary"
                      accessibilityLabel={
                        request
                          ? `Review ${name}'s request: ${request.summary}`
                          : `Open ${name}'s latest work`
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
                  </View>
                </View>
              );
            })}
            {items.length > AWAY_PREVIEW ? (
              <ActionButton
                compact
                variant="secondary"
                label={
                  shown < items.length
                    ? `Show more (${items.length - shown})`
                    : "Show less"
                }
                onPress={() =>
                  setShown((value) =>
                    value < items.length ? value + AWAY_PREVIEW : AWAY_PREVIEW,
                  )
                }
              />
            ) : null}
          </Card>
        ) : !agents.isError ? (
          <Quiet>
            All caught up. Replies and requests from your specialists show up
            here.
          </Quiet>
        ) : null}
      </Section>
      <Section
        title="Your agents"
        action={
          <ActionButton
            label="New agent"
            compact
            variant="secondary"
            disabled={!active}
            onPress={() => {
              if (available()) onNewAgent();
            }}
          />
        }
      >
        {agentOwnerSections(roster).map((section) => (
          <View key={section.id} style={styles.section}>
            <Label accessibilityRole="header" muted style={styles.owner}>
              {section.label}
            </Label>
            {section.agents.map((agent) => (
              <Card key={agent.id}>
                <View style={styles.rowTitle}>
                  <AgentAvatar agent={agent} size="lg" />
                  <Label style={styles.name}>{chatAgentLabel(agent)}</Label>
                  {agent.kind !== "nyxbot" ? (
                    agent.pending_acknowledgements > 0 ? (
                      <SectionBadge label="Needs you" tone="warning" />
                    ) : (
                      <StatusBadge
                        label={agent.status === "running" ? "Running" : "Idle"}
                        variant={
                          agent.status === "running"
                            ? "modeChip"
                            : "decisionExpired"
                        }
                      />
                    )
                  ) : null}
                </View>
                <Label muted style={styles.metadata}>
                  {agent.kind === "nyxbot"
                    ? "Your personal agent"
                    : `@${agent.name}`}
                  {agent.can_use === false ? " · View only" : ""}
                </Label>
                <Label muted numberOfLines={2} style={styles.body}>
                  {agent.description ||
                    (agent.kind === "nyxbot"
                      ? "Full access to your services and account. Delegates to your specialists."
                      : "No role described yet.")}
                </Label>
                <View style={styles.actions}>
                  <ActionButton
                    label="Chat"
                    accessibilityLabel={`Chat with ${chatAgentLabel(agent)}`}
                    compact
                    variant="secondary"
                    disabled={!active || agent.can_use === false}
                    onPress={() => openAgent(agent.id)}
                  />
                </View>
              </Card>
            ))}
          </View>
        ))}
        {!roster.some((agent) => agent.status !== "destroyed") ? (
          <Quiet>
            {agents.isPending
              ? "Loading your agents..."
              : agents.isError
                ? "Your agents are unavailable. Try again above."
                : "No agents yet."}
          </Quiet>
        ) : null}
      </Section>
      <Section
        title="Crews"
        action={
          <ActionButton
            label="New crew"
            compact
            variant="secondary"
            disabled={!active}
            onPress={() => {
              if (available()) onNewCrew();
            }}
          />
        }
      >
        {crews.isError ? (
          <Notice
            tone="error"
            message="Couldn’t refresh your crews. Saved crews may be out of date."
          />
        ) : null}
        {crews.isError ? (
          <ActionButton
            label="Retry crews"
            compact
            variant="secondary"
            disabled={!active}
            onPress={() => {
              if (available()) void crews.refetch();
            }}
          />
        ) : null}
        {groups.map((group) => (
          <Card key={group.id}>
            <View style={styles.rowTitle}>
              <AgentAvatarStack agents={group.members} max={4} />
              <Label style={styles.name}>{group.name}</Label>
              {group.working_agent_ids.length ? (
                <SectionBadge
                  label={`${group.working_agent_ids.length} working`}
                  tone="info"
                />
              ) : null}
            </View>
            <Label muted numberOfLines={2} style={styles.body}>
              {group.owner?.type === "org" ? `${group.owner.name} · ` : ""}
              {group.members.map(chatAgentLabel).join(", ")}
            </Label>
            <View style={styles.actions}>
              {group.last_message_at ? (
                <Label muted style={styles.metadata}>
                  {formatRelativeTimeFromMs(Date.parse(group.last_message_at))}
                </Label>
              ) : null}
              <ActionButton
                label="Open"
                accessibilityLabel={`Open crew ${group.name}`}
                compact
                variant="secondary"
                disabled={!active}
                onPress={() => {
                  if (
                    available() &&
                    latest.current.groups.some((row) => row.id === group.id)
                  )
                    onOpenCrew(group.id);
                }}
              />
            </View>
          </Card>
        ))}
        {!groups.length ? (
          <Quiet>
            {crews.isPending
              ? "Loading crews..."
              : crews.isError
                ? "Your crews are unavailable. Try again above."
                : "Put several agents in one chat. Mention one with @ to ask it directly; NyxBot answers the rest."}
          </Quiet>
        ) : null}
      </Section>
    </ScrollView>
  );
}

function Section({
  title,
  action,
  children,
}: PropsWithChildren<{ title: string; action?: React.ReactNode }>) {
  return (
    <View style={styles.section}>
      <View style={styles.rowTitle}>
        <Label accessibilityRole="header" style={styles.sectionTitle}>
          {title}
        </Label>
        {action}
      </View>
      {children}
    </View>
  );
}
function Quiet({ children }: PropsWithChildren) {
  return (
    <Card>
      <Label muted style={styles.body}>
        {children}
      </Label>
    </Card>
  );
}

const styles = StyleSheet.create({
  content: { paddingTop: 24, paddingBottom: 24, gap: 24 },
  greeting: { flexDirection: "row", alignItems: "center", gap: 12 },
  greetingText: { flex: 1, gap: 6 },
  title: { fontFamily: fonts.displaySemi, fontSize: 24, lineHeight: 30 },
  section: { gap: 12 },
  sectionTitle: { flex: 1, fontFamily: fonts.bodySemi, fontSize: 15 },
  rowTitle: {
    flexDirection: "row",
    alignItems: "center",
    flexWrap: "wrap",
    gap: 8,
  },
  name: {
    flexGrow: 1,
    flexShrink: 1,
    fontFamily: fonts.bodySemi,
    fontSize: 13,
  },
  body: { fontSize: 12, lineHeight: 19 },
  metadata: { flexShrink: 1, fontSize: 11, lineHeight: 17 },
  owner: { fontFamily: fonts.bodySemi, fontSize: 11, letterSpacing: 1 },
  listRow: {
    gap: 8,
    paddingBottom: 12,
    borderBottomWidth: StyleSheet.hairlineWidth,
  },
  actions: {
    flexDirection: "row",
    flexWrap: "wrap",
    gap: 8,
    alignItems: "center",
    justifyContent: "flex-end",
  },
});
