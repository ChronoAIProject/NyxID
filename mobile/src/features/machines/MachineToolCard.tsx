import { useEffect, useRef, useState } from "react";
import { StyleSheet, View } from "react-native";
import { ChevronDown, ChevronRight } from "lucide-react-native";
import { useQuery } from "@tanstack/react-query";
import { z } from "zod";
import {
  SettingsGroup,
  SettingsRow,
} from "../../components/settings/SettingsList";
import { Label, Notice } from "../../components/workspace/WorkspaceUI";
import { useTheme } from "../../theme/ThemeContext";
import { spacing } from "../../theme/designTokens";
import { requestAuthenticatedResponse, requestJson } from "../../lib/api/http";
import { ApiError } from "../../lib/api/ApiError";
import { getAuthEpoch } from "../../lib/auth/authEpoch";
import { MachineDesktopEntries } from "./MachineDesktopEntries";
import { useMachineRuntime } from "./MachineUI";
import { AssistantAttachmentView } from "../assistant/AssistantAttachmentView";
import type { MachineDesktopParams } from "./desktop/desktopRoutes";
import type {
  AssistantMessage,
  AssistantAttachment,
} from "../assistant/assistantTypes";

type Receipt = NonNullable<AssistantMessage["activities"][number]["machine"]>;
const policySchema = z.object({ enabled: z.boolean() });
export function MachineToolCard({
  receipt,
  conversationId,
  groupId,
  attachments = [],
  onOpenDesktop,
}: {
  receipt: Receipt;
  conversationId?: string;
  groupId?: string;
  attachments?: AssistantAttachment[];
  onOpenDesktop?: (params: MachineDesktopParams) => void;
}) {
  const runtime = useMachineRuntime();
  const [ids, setIds] = useState(false);
  const [excerpt, setExcerpt] = useState(false);
  const [preference, setPreference] = useState(false);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState(false);
  const lock = useRef(false);
  const current = useRef({
    active: runtime.active,
    conversationId,
    epoch: runtime.epoch,
  });
  current.current = {
    active: runtime.active,
    conversationId,
    epoch: runtime.epoch,
  };
  const mounted = useRef(true);
  useEffect(
    () => () => {
      mounted.current = false;
    },
    [],
  );
  const root = conversationId
    ? `/assistant/nyxagent/conversations/${encodeURIComponent(conversationId)}`
    : undefined;
  const command = receipt.action.startsWith("command.");
  const policy = useQuery({
    queryKey: [
      "workspace",
      "machine-preview-policy",
      runtime.epoch,
      conversationId,
    ],
    queryFn: async ({ signal }) =>
      policySchema.parse(
        await requestJson(`${root}/machine-preview-policy`, { signal }),
      ),
    enabled: runtime.active && !!root && command && preference,
    retry: false,
  });
  async function changePolicy() {
    if (
      !root ||
      !runtime.active ||
      !policy.data ||
      policy.isFetching ||
      lock.current
    )
      return;
    lock.current = true;
    setSaving(true);
    setError(false);
    try {
      policySchema.parse(
        await requestJson(`${root}/machine-preview-policy`, {
          method: "PUT",
          body: { enabled: !policy.data.enabled },
        }),
      );
      if (alive()) await policy.refetch();
    } catch {
      if (alive()) setError(true);
    } finally {
      lock.current = false;
      if (alive()) setSaving(false);
    }
  }
  function alive() {
    return (
      mounted.current &&
      current.current.active &&
      current.current.conversationId === conversationId &&
      current.current.epoch === runtime.epoch &&
      getAuthEpoch() === runtime.epoch
    );
  }
  const image = attachments.find((a) => a.id === receipt.screenshot_id);
  return (
    <View accessibilityLabel="Machine action" style={styles.receipt}>
      <SettingsGroup inset={false}>
        <SettingsRow
          label={receipt.action.replaceAll(".", " · ").replaceAll("_", " ")}
          value={receipt.status}
        />
        {receipt.exit_code != null ? (
          <SettingsRow label="Exit code" value={String(receipt.exit_code)} />
        ) : null}
        {receipt.bytes != null ? (
          <SettingsRow
            label="Output"
            value={`${receipt.bytes.toLocaleString()} bytes`}
          />
        ) : null}
        <SettingsRow
          label={
            receipt.machine_name?.trim() ||
            `Machine ${receipt.node_id.slice(0, 8)}`
          }
          detail={
            receipt.context_mode === "separated"
              ? "Separate workspace and browser for this agent"
              : "Shared workspace and browser"
          }
        />
        <Disclosure
          label="Correlation IDs"
          open={ids}
          onPress={() => setIds(!ids)}
        />
        {ids
          ? [
              ["Machine", receipt.node_id],
              ["Agent", receipt.agent_id],
              ["Operation", receipt.operation_id],
              ["Job", receipt.job_id],
            ]
              .filter(([, v]) => v)
              .map(([label, value]) => (
                <SettingsRow
                  key={label}
                  label={`${label}: ${value}`}
                  mono
                  selectable
                />
              ))
          : null}
        {receipt.preview_id && root ? (
          <Disclosure
            label={`${excerpt ? "Hide" : "Show"} saved excerpt`}
            open={excerpt}
            onPress={() => setExcerpt(!excerpt)}
          />
        ) : null}
        {command && root ? (
          <Disclosure
            label="Command excerpt preference"
            open={preference}
            onPress={() => setPreference(!preference)}
          />
        ) : null}
      </SettingsGroup>
      {receipt.error_code != null ? (
        <Notice tone="error" message={`Machine error ${receipt.error_code}`} />
      ) : null}
      {image ? (
        <AssistantAttachmentView
          attachment={image}
          conversationId={conversationId}
          groupId={groupId}
        />
      ) : null}
      {receipt.preview_id && root && excerpt ? (
        <CommandExcerpt
          key={`${runtime.epoch}:${root}:${receipt.preview_id}`}
          endpoint={`${root}/attachments/${encodeURIComponent(receipt.preview_id)}`}
          active={runtime.active}
        />
      ) : null}
      {command && root && preference ? (
        <SettingsGroup
          inset={false}
          footer="Off by default. Save bounded, redacted output from future commands in this conversation for up to 30 days. Output can contain private data."
        >
          <SettingsRow
            tone="action"
            label={
              saving
                ? "Saving…"
                : policy.data?.enabled
                  ? "Stop saving excerpts"
                  : "Save future excerpts"
            }
            disabled={
              !runtime.active || saving || !policy.data || policy.isFetching
            }
            onPress={() => void changePolicy()}
          />
        </SettingsGroup>
      ) : null}
      {command && root && preference && error ? (
        <Notice tone="error" message="Could not change excerpt preference." />
      ) : null}
      {command && root && preference && policy.isError ? (
        <Notice tone="error" message="Could not load excerpt preference." />
      ) : null}
      {onOpenDesktop ? (
        <MachineDesktopEntries
          nodeId={receipt.node_id}
          conversationId={conversationId}
          onOpen={onOpenDesktop}
        />
      ) : null}
    </View>
  );
}
function Disclosure({
  label,
  open,
  onPress,
}: {
  label: string;
  open: boolean;
  onPress: () => void;
}) {
  const { colors } = useTheme();
  const Icon = open ? ChevronDown : ChevronRight;
  return (
    <SettingsRow
      label={label}
      accessibilityLabel={label}
      accessibilityState={{ expanded: open }}
      accessory={<Icon size={18} color={colors.textMuted} />}
      onPress={onPress}
    />
  );
}
export function CommandExcerpt({
  endpoint,
  active,
}: {
  endpoint: string;
  active: boolean;
}) {
  const [text, setText] = useState<string>();
  const [failed, setFailed] = useState(false);
  const [expired, setExpired] = useState(false);
  useEffect(() => {
    if (!active) return;
    const controller = new AbortController();
    const epoch = getAuthEpoch();
    setText(undefined);
    setFailed(false);
    setExpired(false);
    void requestAuthenticatedResponse(endpoint, { signal: controller.signal })
      .then((r) => r.text())
      .then((value) => {
        if (!controller.signal.aborted && epoch === getAuthEpoch())
          setText(value.slice(0, 4096));
      })
      .catch((cause: unknown) => {
        if (!controller.signal.aborted && epoch === getAuthEpoch()) {
          setFailed(true);
          setExpired(cause instanceof ApiError && cause.errorCode === 12101);
        }
      });
    return () => controller.abort();
  }, [endpoint, active]);
  return (
    <Label selectable mono accessibilityLiveRegion="polite">
      {!active
        ? "Excerpt unavailable."
        : expired
          ? "Command excerpt expired per retention policy."
          : failed
            ? "Excerpt unavailable."
            : text === undefined
              ? "Loading excerpt…"
              : text}
    </Label>
  );
}
const styles = StyleSheet.create({
  receipt: { gap: spacing.sm },
});
