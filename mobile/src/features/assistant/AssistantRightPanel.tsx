import { useAppLanguage } from "../../i18n/LanguageProvider";
import { translateApp, appLocale } from "../../i18n/appTranslation";
import { ScrollView, StyleSheet, View } from "react-native";
import { useQuery } from "@tanstack/react-query";
import { AgentAvatar } from "../../components/AgentAvatar";
import { Card, Label, Notice } from "../../components/workspace/WorkspaceUI";
import {
  workspacePanelContent,
  RIGHT_PANEL_WIDTH,
} from "../../components/workspace/workspaceLayout";
import { agentsApi } from "../../lib/api/agentsApi";
import { useGroupAvailability } from "../groups/useGroupRuntime";
import type { AssistantAgent } from "../agents/agentContracts";
import { NyxBotHomePanel } from "./NyxBotHome";
import { useTheme } from "../../theme/ThemeContext";
import { fonts } from "../../theme/designTokens";
import { useRailScrollIndicator } from "../../components/workspace/workspaceFloatingInset";
type RightPanelAgent = {
  id: string;
  name: string;
  kind: "nyxbot" | "specialist" | "unknown";
} & Partial<
  Pick<
    AssistantAgent,
    | "display_name"
    | "description"
    | "services"
    | "account_read"
    | "status"
    | "last_reply"
  >
>;

export function AssistantRightPanel({
  home,
  agent,
  conversation,
  onChat,
  onNewAgent,
}: {
  home: boolean;
  agent?: RightPanelAgent;
  conversation?: {
    last_message_at?: string;
    message_count?: number;
  };
  onChat: (
    agentId: string,
    conversationId?: string,
    requestId?: string,
  ) => void;
  onNewAgent: () => void;
}) {
  const railIndicator = useRailScrollIndicator();
  useAppLanguage();
  const { colors } = useTheme();
  const runtime = useGroupAvailability();
  const detail = useQuery({
    queryKey: ["workspace", "agents", "panel", runtime.epoch, agent?.id],
    queryFn: ({ signal }) => agentsApi.detail(agent!.id, signal),
    enabled: runtime.active && !home && !!agent?.id,
    retry: false,
    refetchInterval: runtime.active ? 30_000 : false,
  });
  const resolvedAgent = detail.data?.agent ?? agent;
  const services = useQuery({
    queryKey: [
      "workspace",
      "grant-services",
      runtime.epoch,
      detail.data?.agent.owner_id,
    ],
    queryFn: ({ signal }) =>
      agentsApi.grantServices(
        detail.data?.agent.owner_kind === "org"
          ? detail.data.agent.owner_id
          : undefined,
        signal,
      ),
    enabled:
      runtime.active &&
      !home &&
      !!detail.data &&
      !!resolvedAgent?.services?.length,
    retry: false,
  });
  const content = workspacePanelContent(home, !!resolvedAgent);
  if (content === "home")
    return (
      <View style={styles.rightPanel}>
        <NyxBotHomePanel onChat={onChat} onNewAgent={onNewAgent} />
      </View>
    );
  if (content === "loading")
    return (
      <View style={styles.rightPanel}>
        <Notice message={translateApp("Loading agent details…")} />
      </View>
    );
  const name =
    resolvedAgent?.display_name?.trim() ||
    resolvedAgent?.name ||
    translateApp("Agent");
  return (
    <ScrollView
      testID="assistant-right-panel"
      style={styles.rightPanel}
      contentContainerStyle={styles.rightPanelContent}
      keyboardShouldPersistTaps="handled"
      {...railIndicator}
    >
      {detail.isPending && !detail.data ? (
        <Notice message={translateApp("Loading agent details…")} />
      ) : null}
      {detail.isError ? (
        <Notice
          message={translateApp(
            "Agent details could not be refreshed. Showing saved information.",
          )}
        />
      ) : null}
      {!runtime.online ? (
        <Notice
          message={translateApp("You’re offline. Showing saved activity.")}
        />
      ) : null}
      <Card style={styles.detailCard}>
        {agent ? <AgentAvatar agent={agent} size="xl" /> : null}
        <Label accessibilityRole="header" style={styles.panelTitle}>
          {name}
        </Label>
        {resolvedAgent?.description ? (
          <Label muted>{resolvedAgent.description}</Label>
        ) : null}
      </Card>
      <Card style={styles.detailCard}>
        <Label accessibilityRole="header" style={styles.panelSectionTitle}>
          {translateApp("Access")}
        </Label>
        <Label muted>
          {resolvedAgent?.account_read
            ? translateApp("Account access enabled")
            : translateApp("Uses only the services granted to this agent.")}
        </Label>
        {services.isError ? (
          <Label muted>
            {translateApp("Service details are unavailable.")}
          </Label>
        ) : null}
        {resolvedAgent?.services?.length ? (
          <Label muted>
            {resolvedAgent.services
              .map(
                (id) =>
                  services.data?.find(
                    (service) => service.id === id || service.slug === id,
                  )?.label ?? id,
              )
              .join(", ")}
          </Label>
        ) : null}
      </Card>
      <Card style={styles.detailCard}>
        <Label accessibilityRole="header" style={styles.panelSectionTitle}>
          {translateApp("Recent activity")}
        </Label>
        <Label muted>
          {conversation?.message_count
            ? translateApp("{{count}} messages", {
                count: conversation.message_count,
              })
            : translateApp("No messages yet.")}
        </Label>
        {conversation?.last_message_at ? (
          <Label muted>{formatDate(conversation.last_message_at)}</Label>
        ) : null}
        {resolvedAgent?.last_reply?.text ? (
          <Label numberOfLines={4} muted>
            {resolvedAgent.last_reply.text}
          </Label>
        ) : null}
        {detail.data?.threads.slice(0, 3).map((thread) => (
          <View key={thread.id} style={{ gap: 4 }}>
            <Label numberOfLines={2}>{thread.title}</Label>
            <Label muted>{formatDate(thread.last_message_at)}</Label>
          </View>
        ))}
      </Card>
      <View
        style={[styles.panelAccent, { backgroundColor: colors.groupedFill }]}
      >
        <Label muted>
          {translateApp(
            "Open Chat details for permissions, memory and history.",
          )}
        </Label>
      </View>
    </ScrollView>
  );
}

const formatDate = (value: string) =>
  new Date(value).toLocaleString(appLocale(), {
    month: "short",
    day: "numeric",
    hour: "2-digit",
    minute: "2-digit",
  });
const styles = StyleSheet.create({
  panelAccent: { padding: 12, borderRadius: 12 },
  rightPanel: {
    width: RIGHT_PANEL_WIDTH,
    flex: 1,
    minHeight: 0,
    flexShrink: 0,
  },
  rightPanelContent: { paddingHorizontal: 12, paddingVertical: 24, gap: 12 },
  detailCard: { gap: 8, padding: 12 },
  panelTitle: { fontFamily: fonts.displaySemi, fontSize: 18, lineHeight: 24 },
  panelSectionTitle: { fontFamily: fonts.bodySemi, fontSize: 13 },
});
