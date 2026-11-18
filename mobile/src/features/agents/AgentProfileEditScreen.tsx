import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useIsFocused } from "@react-navigation/native";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { ZodError } from "zod";
import type { ContentScreenProps } from "../../app/navigationTypes";
import { useWorkspaceBack } from "../../app/WorkspaceHeader";
import {
  SettingsGroup,
  SettingsPage,
  SettingsTextField,
} from "../../components/settings/SettingsList";
import { SettingsHeaderButton } from "../../components/settings/SettingsHeaderButton";
import { SkeletonRows } from "../../components/Skeleton";
import { Notice } from "../../components/workspace/WorkspaceUI";
import { useNetworkStatus } from "../../hooks/useNetworkStatus";
import { agentsApi } from "../../lib/api/agentsApi";
import {
  agentProfile,
  buildAgentPatch,
  profileHasConflict,
  type AssistantAgent,
} from "./agentContracts";

export type AgentProfileField =
  "name" | "display_name" | "description" | "persona";

const ROOT = ["workspace", "agents"] as const;
const COPY: Record<
  AgentProfileField,
  {
    title: string;
    placeholder: string;
    maxLength: number;
    multiline?: boolean;
    footer: string;
  }
> = {
  name: {
    title: "Handle",
    placeholder: "researcher",
    maxLength: 32,
    footer:
      "Use 1–32 lowercase letters, digits or hyphens. People and agents @mention this handle.",
  },
  display_name: {
    title: "Display name",
    placeholder: "Optional friendly name",
    maxLength: 40,
    footer: "Shown instead of the handle in chats and crews.",
  },
  description: {
    title: "Purpose",
    placeholder: "What should this agent do?",
    maxLength: 2048,
    multiline: true,
    footer:
      "What this agent is for. Its granted services still decide what it can do.",
  },
  persona: {
    title: "Personality and tone",
    placeholder: "Warm, concise, explains its decisions",
    maxLength: 2000,
    multiline: true,
    footer: "Personality changes its tone, not its authority.",
  },
};

/** The row and page title; NyxBot's description is its owner's notes. */
export function agentProfileFieldTitle(
  field: AgentProfileField,
  nyxbot = false,
) {
  return field === "description" && nyxbot ? "Notes" : COPY[field].title;
}

const errorMessage = (cause: unknown) =>
  cause instanceof ZodError
    ? (cause.issues[0]?.message ?? "Check this value.")
    : cause instanceof Error
      ? cause.message
      : "Could not complete this action. Try again.";

/**
 * Agent › one profile value on its own page, Save in the header. The value
 * shown when the page opened is the baseline: a change made elsewhere since
 * then is refused instead of being overwritten.
 */
export function AgentProfileEditScreen({
  navigation,
  route,
}: ContentScreenProps<"AgentProfileEdit">) {
  const { agentId, field } = route.params;
  const copy = COPY[field];
  const queryClient = useQueryClient();
  const focused = useIsFocused();
  const { isConnected } = useNetworkStatus();
  const detail = useQuery({
    queryKey: [...ROOT, "detail", agentId],
    queryFn: ({ signal }) => agentsApi.detail(agentId, signal),
    retry: false,
    enabled: focused,
  });
  const [baseline, setBaseline] = useState<AssistantAgent>();
  const [value, setValue] = useState<string>();
  const [busy, setBusy] = useState(false);
  const [done, setDone] = useState(false);
  const [error, setError] = useState<string>();
  const lock = useRef(false);
  const mounted = useRef(true);
  useEffect(() => {
    mounted.current = true;
    return () => {
      mounted.current = false;
    };
  }, []);
  useEffect(() => {
    if (!baseline && detail.data) setBaseline(detail.data.agent);
  }, [baseline, detail.data]);
  const nyxbot = baseline?.kind === "nyxbot";
  const title = agentProfileFieldTitle(field, nyxbot);
  const saved = baseline ? agentProfile(baseline)[field] : "";
  const draft = value ?? saved;
  const editable =
    !!baseline &&
    baseline.can_maintain === true &&
    baseline.status !== "destroyed" &&
    !(field === "name" && nyxbot);
  const dirty = !done && value !== undefined && value !== saved;
  const canSave = editable && dirty && isConnected && !busy;
  async function save() {
    if (lock.current || !canSave || !baseline) return;
    lock.current = true;
    setBusy(true);
    setError(undefined);
    try {
      const patch = buildAgentPatch(baseline, {
        ...agentProfile(baseline),
        [field]: draft,
      });
      if (Object.keys(patch).length) {
        const latest = (await agentsApi.detail(agentId)).agent;
        if (latest.can_maintain !== true || latest.status === "destroyed")
          throw new Error(
            "This agent can no longer be changed. Reload to review its current state.",
          );
        if (profileHasConflict(baseline, latest, patch))
          throw new Error(
            "This profile changed elsewhere. Go back and review those changes before saving.",
          );
        await agentsApi.update(agentId, patch);
      }
      await queryClient.invalidateQueries({ queryKey: ROOT });
      if (mounted.current) setDone(true);
    } catch (cause) {
      if (mounted.current) setError(errorMessage(cause));
    } finally {
      lock.current = false;
      if (mounted.current) setBusy(false);
    }
  }
  const saveRef = useRef(save);
  saveRef.current = save;
  const headerRight = useCallback(
    () => (
      <SettingsHeaderButton
        enabled={canSave}
        busy={busy}
        onPress={() => void saveRef.current()}
      />
    ),
    [canSave, busy],
  );
  const options = useMemo(() => ({ title, headerRight }), [title, headerRight]);
  useWorkspaceBack({
    active: false,
    defaultRole: "back",
    onBack: navigation.goBack,
    disabled: busy,
    options,
    discardOnClose: dirty
      ? {
          title: "Discard changes?",
          message: "Your unsaved profile change will be lost.",
        }
      : undefined,
  });
  useEffect(() => {
    if (done && !busy) navigation.goBack();
  }, [done, busy, navigation]);
  return (
    <SettingsPage>
      {!isConnected && (
        <Notice message="You're offline. Connect to save changes." />
      )}
      {detail.isPending && (
        <SkeletonRows
          rows={2}
          icon={false}
          title
          accessibilityLabel="Loading agent"
        />
      )}
      {detail.isError && !baseline && (
        <Notice tone="error" message={errorMessage(detail.error)} />
      )}
      {baseline && !editable && (
        <Notice
          message={
            field === "name" && nyxbot
              ? "NyxBot's handle cannot be changed."
              : "This agent can no longer be changed here."
          }
        />
      )}
      {error && <Notice tone="error" message={error} />}
      <SettingsGroup inset={false} footer={copy.footer}>
        <SettingsTextField
          accessibilityLabel={title}
          value={draft}
          onChangeText={(text) => {
            setValue(field === "name" ? text.toLowerCase() : text);
            setError(undefined);
          }}
          placeholder={copy.placeholder}
          maxLength={copy.maxLength}
          multiline={copy.multiline}
          autoFocus
          editable={editable && !busy && !done}
          {...(field === "name"
            ? { autoCapitalize: "none" as const, autoCorrect: false }
            : { autoCapitalize: "sentences" as const })}
          {...(copy.multiline
            ? {
                clearButtonMode: "never" as const,
                style: { minHeight: 120, textAlignVertical: "top" as const },
              }
            : {
                returnKeyType: "done" as const,
                onSubmitEditing: () => {
                  if (canSave) void save();
                },
              })}
        />
      </SettingsGroup>
    </SettingsPage>
  );
}
