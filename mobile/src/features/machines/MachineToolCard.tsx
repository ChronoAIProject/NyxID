import { useEffect, useRef, useState } from "react";
import { Pressable, View } from "react-native";
import { useQuery } from "@tanstack/react-query";
import { z } from "zod";
import {
  Card,
  Label,
  ActionButton,
} from "../../components/workspace/WorkspaceUI";
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
    <Card>
      <View accessibilityLabel="Machine action">
        <Label>
          {receipt.action.replaceAll(".", " · ").replaceAll("_", " ")} ·{" "}
          {receipt.status}
        </Label>
        {receipt.exit_code != null ? (
          <Label muted>Exit {receipt.exit_code}</Label>
        ) : null}
        {receipt.bytes != null ? (
          <Label muted>{receipt.bytes.toLocaleString()} bytes</Label>
        ) : null}
        <Label muted>
          {receipt.machine_name?.trim() ||
            `Machine ${receipt.node_id.slice(0, 8)}`}{" "}
          ·{" "}
          {receipt.context_mode === "separated"
            ? "Separate workspace and browser for this agent"
            : "Shared workspace and browser"}
        </Label>
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
                <Label key={label} selectable muted>
                  {label}: {value}
                </Label>
              ))
          : null}
        {receipt.error_code != null ? (
          <Label accessibilityRole="alert">
            Machine error {receipt.error_code}
          </Label>
        ) : null}
        {image ? (
          <AssistantAttachmentView
            attachment={image}
            conversationId={conversationId}
            groupId={groupId}
          />
        ) : null}
        {receipt.preview_id && root ? (
          <>
            <Disclosure
              label={`${excerpt ? "Hide" : "Show"} saved excerpt`}
              open={excerpt}
              onPress={() => setExcerpt(!excerpt)}
            />
            {excerpt ? (
              <CommandExcerpt
                key={`${runtime.epoch}:${root}:${receipt.preview_id}`}
                endpoint={`${root}/attachments/${encodeURIComponent(receipt.preview_id)}`}
                active={runtime.active}
              />
            ) : null}
          </>
        ) : null}
        {command && root ? (
          <>
            <Disclosure
              label="Command excerpt preference"
              open={preference}
              onPress={() => setPreference(!preference)}
            />
            {preference ? (
              <>
                <Label muted>
                  Off by default. Save bounded, redacted output from future
                  commands in this conversation for up to 30 days. Output can
                  contain private data.
                </Label>
                <ActionButton
                  compact
                  variant="secondary"
                  label={
                    policy.data?.enabled
                      ? "Stop saving excerpts"
                      : "Save future excerpts"
                  }
                  loading={saving}
                  disabled={
                    !runtime.active ||
                    saving ||
                    !policy.data ||
                    policy.isFetching
                  }
                  onPress={() => void changePolicy()}
                />
                {error ? (
                  <Label accessibilityRole="alert">
                    Could not change excerpt preference.
                  </Label>
                ) : null}
                {policy.isError ? (
                  <Label accessibilityRole="alert">
                    Could not load excerpt preference.
                  </Label>
                ) : null}
              </>
            ) : null}
          </>
        ) : null}
        {onOpenDesktop ? (
          <MachineDesktopEntries
            nodeId={receipt.node_id}
            conversationId={conversationId}
            onOpen={onOpenDesktop}
          />
        ) : null}
      </View>
    </Card>
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
  return (
    <Pressable
      accessibilityRole="button"
      accessibilityLabel={label}
      accessibilityState={{ expanded: open }}
      style={{ minHeight: 44, justifyContent: "center" }}
      onPress={onPress}
    >
      <Label muted>
        {open ? "⌄" : "›"} {label}
      </Label>
    </Pressable>
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
