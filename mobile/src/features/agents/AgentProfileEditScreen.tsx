import { localizedError } from "../../i18n/localizedError";
import { useLocalizedState } from "../../i18n/useLocalizedState";
import { useAppLanguage } from "../../i18n/LanguageProvider";
import { translateApp } from "../../i18n/appTranslation";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useIsFocused } from "@react-navigation/native";
import { isAgentAccessRefusal } from "./agentAccess";
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
  | "name" | "display_name" | "description" | "persona";

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
  get name() {
    return {
      get title() {
        return translateApp("Handle");
      },
      get placeholder() {
        return translateApp("researcher");
      },
    maxLength: 32,
      get footer() {
        return translateApp(
          "Use 1–32 lowercase letters, digits or hyphens. People and agents @mention this handle.",
        );
      },
    };
  },
  get display_name() {
    return {
      get title() {
        return translateApp("Display name");
      },
      get placeholder() {
        return translateApp("Optional friendly name");
      },
    maxLength: 40,
      get footer() {
        return translateApp("Shown instead of the handle in chats and crews.");
      },
  };
  },
  get description() {
    return {
      get title() {
        return translateApp("Purpose");
      },
      get placeholder() {
        return translateApp("What should this agent do?");
      },
    maxLength: 2048,
    multiline: true,
      get footer() {
        return translateApp(
          "What this agent is for. Its granted services still decide what it can do.",
        );
      },
    };
  },
  get persona() {
    return {
      get title() {
        return translateApp("Personality and tone");
      },
      get placeholder() {
        return translateApp("Warm, concise, explains its decisions");
      },
    maxLength: 2000,
    multiline: true,
      get footer() {
        return translateApp("Personality changes its tone, not its authority.");
      },
    };
  },
};

/** The row and page title; NyxBot's description is its owner's notes. */
export function agentProfileFieldTitle(
  field: AgentProfileField,
  nyxbot = false,
) {
  return field === "description" && nyxbot ? translateApp("Notes")
    : COPY[field].title;
}

const errorMessage = (cause: unknown) =>
  cause instanceof ZodError
    ? (cause.issues[0]?.message ?? translateApp("Check this value."))
    : cause instanceof Error
      ? cause.message
      : translateApp("Could not complete this action. Try again.");

/**
 * Agent › one profile value on its own page, Save in the header. The value
 * shown when the page opened is the baseline: a change made elsewhere since
 * then is refused instead of being overwritten.
 */
export function AgentProfileEditScreen({
  navigation,
  route,
}: ContentScreenProps<"AgentProfileEdit">) {
  useAppLanguage();
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
  const [error, setError] = useLocalizedState<string>();
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
    !isAgentAccessRefusal(detail.error) &&
    detail.data?.agent.can_maintain === true &&
    detail.data.agent.status !== "destroyed" &&
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
        if (latest.can_maintain !== true || latest.status === "destroyed") {
          await queryClient.invalidateQueries({ queryKey: ROOT });
          if (mounted.current) setDone(true);
          throw localizedError("This agent can no longer be changed. Reload to review its current state.");
        }
        if (profileHasConflict(baseline, latest, patch))
          throw localizedError("This profile changed elsewhere. Go back and review those changes before saving.");
        await agentsApi.update(agentId, patch);
      }
      await queryClient.invalidateQueries({ queryKey: ROOT });
      if (mounted.current) setDone(true);
    } catch (cause) {
      if (isAgentAccessRefusal(cause)) await queryClient.invalidateQueries({ queryKey: ROOT });
      if (mounted.current) setError(() => (errorMessage(cause)));
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
          title: translateApp("Discard changes?"),
          message: translateApp("Your unsaved profile change will be lost."),
        }
      : undefined,
  });
  useEffect(() => {
    const lostAccess = baseline?.can_maintain === true && (detail.data?.agent.can_maintain === false || isAgentAccessRefusal(detail.error));
    if ((done || lostAccess) && !busy) navigation.goBack();
  }, [done, busy, baseline, detail.data, detail.error, navigation]);
  return (
    <SettingsPage>
      {!isConnected && (
        <Notice message={translateApp("You're offline. Connect to save changes.")}
        />
      )}
      {detail.isPending && (
        <SkeletonRows
          rows={2}
          icon={false}
          title
          accessibilityLabel={translateApp("Loading agent")}
        />
      )}
      {detail.isError && !baseline && (
        <Notice tone="error" message={errorMessage(detail.error)} />
      )}
      {baseline && !editable && (
        <Notice
          message={
            field === "name" && nyxbot
              ? translateApp("NyxBot's handle cannot be changed.")
              : translateApp("This agent can no longer be changed here.")
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
