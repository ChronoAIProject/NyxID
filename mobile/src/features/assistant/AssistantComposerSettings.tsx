import { useEffect, useRef, useState } from "react";
import { Modal, Pressable, ScrollView, View } from "react-native";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { Check, ChevronDown, X } from "lucide-react-native";
import { Label } from "../../components/workspace/WorkspaceUI";
import {
  workspaceCloseControlStyle,
  WORKSPACE_CLOSE_ICON_SIZE,
} from "../../components/workspace/workspaceCloseControl";
import { useTheme } from "../../theme/ThemeContext";
import { assistantApi } from "../../lib/api/assistantApi";
import { getAuthEpoch } from "../../lib/auth/authEpoch";
import { useGroupAvailability } from "../groups/useGroupRuntime";
import {
  COMPOSER_ACCESS_LABEL,
  COMPOSER_ACCESS_DESCRIPTION,
  type ComposerAccessMode,
} from "./composerSettings";

type Props = {
  scope: string;
  agentOverride?: ComposerAccessMode | null;
  activeTurn: boolean;
  disabled: boolean;
  visible: boolean;
  onOpenChange: (open: boolean) => void;
  hideTrigger?: boolean;
};

/** Owner permissions are separate from the human-only per-agent override editor. */
export function AssistantComposerSettings(props: Props) {
  const { colors } = useTheme();
  const availability = useGroupAvailability();
  const queryClient = useQueryClient();
  const epoch = getAuthEpoch();
  const settingsKey = ["assistant", "composer-settings", epoch];
  const settings = useQuery({
    queryKey: settingsKey,
    queryFn: ({ signal }) => assistantApi.settings(signal),
    enabled: availability.active,
    retry: false,
  });
  const [pending, setPending] = useState(false);
  const [error, setError] = useState<string>();
  const lock = useRef(false);
  const mounted = useRef(true);
  const latest = useRef({ props, availability });
  latest.current = { props, availability };
  useEffect(() => {
    mounted.current = true;
    return () => {
      mounted.current = false;
    };
  }, []);
  useEffect(() => {
    setError(undefined);
    props.onOpenChange(false);
  }, [props.scope]);
  const mode = props.agentOverride ?? settings.data?.access_mode;
  const accessLabel = mode ? COMPOSER_ACCESS_LABEL[mode] : "Permissions";
  const title = "Permissions";
  const permissionLocked =
    props.activeTurn || !!props.agentOverride || pending || !settings.data;
  const unavailable = props.disabled || !availability.active;
  async function change(value: ComposerAccessMode) {
    const state = latest.current;
    if (
      lock.current ||
      !state.availability.available() ||
      state.props.disabled ||
      state.props.activeTurn ||
      getAuthEpoch() !== epoch
    )
      return;
    if (state.props.agentOverride || !settings.data || value === mode) return;
    const scope = state.props.scope;
    const current = () =>
      mounted.current &&
      getAuthEpoch() === epoch &&
      latest.current.props.scope === scope;
    lock.current = true;
    setPending(true);
    setError(undefined);
    try {
      // The web composer updates directly, including Full access (no confirmation).
      const result = await assistantApi.updateAccessMode(value);
      if (current()) queryClient.setQueryData(settingsKey, result);
    } catch (cause) {
      if (current())
        setError(
          cause instanceof Error
            ? cause.message
            : "Could not change permissions.",
        );
    } finally {
      lock.current = false;
      if (mounted.current) setPending(false);
    }
  }
  function row(
    value: ComposerAccessMode,
    selected: boolean,
    disabled: boolean,
    onPress: () => void,
  ) {
    const label = COMPOSER_ACCESS_LABEL[value];
    return (
      <Pressable
        key={label}
        accessibilityRole="radio"
        accessibilityLabel={label}
        accessibilityState={{
          checked: selected,
          disabled: disabled || unavailable,
        }}
        disabled={disabled || unavailable}
        onPress={onPress}
        style={{
          minHeight: 44,
          paddingHorizontal: 12,
          paddingVertical: 10,
          flexDirection: "row",
          alignItems: "center",
          gap: 8,
          borderRadius: 8,
          backgroundColor: selected ? colors.cardSoft : "transparent",
          opacity: disabled || unavailable ? 0.5 : 1,
        }}
      >
        <View style={{ flex: 1, gap: 4 }}>
          <Label style={{ fontSize: 14 }}>{label}</Label>
          <Label muted style={{ fontSize: 12 }}>
            {COMPOSER_ACCESS_DESCRIPTION[value]}
          </Label>
        </View>
        {selected ? <Check size={16} color={colors.textSecondary} /> : null}
      </Pressable>
    );
  }
  return (
    <>
      {!props.hideTrigger && (
        <Pressable
          accessibilityRole="button"
          accessibilityLabel={`Permissions: ${accessLabel}${props.agentOverride ? " (this agent)" : ""}`}
          disabled={unavailable}
          onPress={() => props.onOpenChange(true)}
          style={{
            minHeight: 44,
            minWidth: 44,
            maxWidth: 180,
            flexShrink: 1,
            flexDirection: "row",
            alignItems: "center",
            gap: 4,
            paddingHorizontal: 6,
            opacity: unavailable ? 0.5 : 1,
          }}
        >
          <Label
            numberOfLines={1}
            maxFontSizeMultiplier={1.3}
            style={{ fontSize: 12, flexShrink: 1 }}
          >
            {accessLabel}
          </Label>
          <ChevronDown size={12} color={colors.textSecondary} />
        </Pressable>
      )}
      <Modal
        visible={
          props.visible && availability.focused && availability.foreground
        }
        transparent
        animationType="fade"
        onRequestClose={() => props.onOpenChange(false)}
      >
        <View
          style={{
            flex: 1,
            justifyContent: "center",
            padding: 20,
            backgroundColor: colors.sheetScrim,
          }}
          accessibilityViewIsModal
        >
          <View
            style={{
              backgroundColor: colors.card,
              borderColor: colors.border,
              borderWidth: 1,
              borderRadius: 16,
              maxHeight: "85%",
              width: "100%",
              maxWidth: 520,
              alignSelf: "center",
              overflow: "hidden",
            }}
          >
            <View
              style={{
                flexDirection: "row",
                alignItems: "center",
                padding: 12,
                gap: 8,
              }}
            >
              <Label
                accessibilityRole="header"
                numberOfLines={1}
                maxFontSizeMultiplier={1.25}
                style={{ fontSize: 18, flex: 1 }}
              >
                {title}
              </Label>
              <Pressable
                accessibilityRole="button"
                accessibilityLabel={`Close ${title.toLowerCase()}`}
                style={workspaceCloseControlStyle(colors.cardSoft)}
                onPress={() => props.onOpenChange(false)}
              >
                <X
                  size={WORKSPACE_CLOSE_ICON_SIZE}
                  color={colors.textSecondary}
                />
              </Pressable>
            </View>
            <ScrollView
              contentContainerStyle={{ padding: 12, gap: 4 }}
              keyboardShouldPersistTaps="handled"
            >
              {error ? (
                <Label
                  accessibilityRole="alert"
                  style={{ color: colors.danger }}
                >
                  {error}
                </Label>
              ) : null}
              {props.agentOverride ? (
                <Label muted>
                  This agent uses {accessLabel}. Its override is read-only here.
                </Label>
              ) : null}
              {(["autopilot", "nyxmode", "ask_every_time"] as const).map(
                (value) =>
                  row(
                    value,
                    mode === value,
                    permissionLocked,
                    () => void change(value),
                  ),
              )}
              {settings.isError ? (
                <Pressable
                  accessibilityRole="button"
                  style={{ minHeight: 44, justifyContent: "center" }}
                  disabled={unavailable}
                  onPress={() => void settings.refetch()}
                >
                  <Label>Retry loading permissions</Label>
                </Pressable>
              ) : null}
            </ScrollView>
          </View>
        </View>
      </Modal>
    </>
  );
}

export function useComposerAccessMode(override?: ComposerAccessMode | null) {
  const availability = useGroupAvailability();
  const epoch = getAuthEpoch();
  const settings = useQuery({
    queryKey: ["assistant", "composer-settings", epoch],
    queryFn: ({ signal }) => assistantApi.settings(signal),
    enabled: availability.active,
    retry: false,
  });
  return override ?? settings.data?.access_mode;
}
