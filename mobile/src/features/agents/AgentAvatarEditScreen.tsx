import { VoiceRippleBanner } from "../../components/avatar/VoiceRippleBanner";
import {
  drawAvatar,
  FACE_GEOMETRY,
} from "../../../../shared/agent-avatars/draw";
import { useSharedValue } from "react-native-reanimated";
import { getAuthEpoch } from "../../lib/auth/authEpoch";
import { incompatibilityMessage } from "../../../../shared/agent-avatars/editor";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { Pressable, StyleSheet, View } from "react-native";
import { DynamicTypeText as Text } from "../../components/DynamicTypeText";
import { useIsFocused } from "@react-navigation/native";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { Dice5, RotateCcw } from "lucide-react-native";
import { ZodError } from "zod";
import { File, Paths } from "expo-file-system";
import { shareAsync } from "expo-sharing";
import { pngAvatarAsync } from "../../../../shared/agent-avatars/export";
import type { ContentScreenProps } from "../../app/navigationTypes";
import { useWorkspaceBack } from "../../app/WorkspaceHeader";
import { AgentAvatar } from "../../components/AgentAvatar";
import { AvatarStill } from "../../components/avatar/AvatarArt";
import {
  SettingsGroup,
  SettingsPage,
  SettingsRow,
} from "../../components/settings/SettingsList";
import { SettingsHeaderButton } from "../../components/settings/SettingsHeaderButton";
import { SkeletonRows } from "../../components/Skeleton";
import { Notice } from "../../components/workspace/WorkspaceUI";
import { useNetworkStatus } from "../../hooks/useNetworkStatus";
import { useAppLanguage } from "../../i18n/LanguageProvider";
import { agentsApi } from "../../lib/api/agentsApi";
import { useAgentAvatarsEnabled } from "../../lib/avatars/agentAvatarsFlag";
import {
  AVATAR_MOODS,
  avatarChoiceLabel,
  avatarChoices,
  avatarMoodLabel,
  canEditAvatar,
  chooseAvatar,
  isValidAvatar,
  reactionFor,
  reactionLabel,
  randomAvatar,
  resolveAvatar,
  type AvatarConfig,
  type AvatarField,
  type AvatarMood,
  type AvatarTheme,
} from "../../lib/avatars/agentAvatars";
import { editableAvatarSchema } from "../../lib/avatars/avatarSchema";
import { useTheme } from "../../theme/ThemeContext";
import { fonts } from "../../theme/designTokens";
import type { AssistantAgent } from "./agentContracts";
import {
  AVATAR_EDITOR_SLOTS,
  avatarChangedElsewhere,
  avatarEditorState,
} from "./avatarEditorModel";

const ROOT = ["workspace", "agents"] as const;

const errorMessage = (cause: unknown) =>
  cause instanceof Error && cause.message
    ? cause.message
    : "Could not save the avatar. Try again.";

/**
 * Agent › Avatar on its own page, Save in the header: a large live preview
 * with every expression, Randomise, Reset to default and one picker per
 * part and colour. Matches the web editor in the agent details drawer. The
 * avatar shown when the page opened is the baseline: a change made
 * elsewhere since then is refused instead of being overwritten.
 */
export function AgentAvatarEditScreen(
  props: ContentScreenProps<"AgentAvatarEdit">,
) {
  return (
    <ScopedAgentAvatarEditScreen
      key={`${getAuthEpoch()}:${props.route.params.agentId}`}
      {...props}
    />
  );
}
function ScopedAgentAvatarEditScreen({
  navigation,
  route,
}: ContentScreenProps<"AgentAvatarEdit">) {
  const { agentId } = route.params;
  const { t } = useAppLanguage();
  const { mode } = useTheme();
  const previewLevel = useSharedValue(0.6);
  const queryClient = useQueryClient();
  const focused = useIsFocused();
  const enabled = useAgentAvatarsEnabled();
  const { isConnected } = useNetworkStatus();
  const detail = useQuery({
    queryKey: [...ROOT, "detail", agentId],
    queryFn: ({ signal }) => agentsApi.detail(agentId, signal),
    retry: false,
    enabled: focused,
  });
  const [baseline, setBaseline] = useState<AssistantAgent>();
  const [draft, setDraft] = useState<AvatarConfig>();
  const [preview, setPreview] = useState<AvatarMood>("idle");
  const [previewEvent, setPreviewEvent] = useState(0);
  const [busy, setBusy] = useState(false);
  const [exporting, setExporting] = useState(false);
  const exportLock = useRef(false);
  const [done, setDone] = useState(false);
  const [error, setError] = useState<string>();
  const lock = useRef(false);
  const mounted = useRef(true);
  const epoch = useRef(getAuthEpoch());
  const guard = () => {
    if (getAuthEpoch() !== epoch.current || !mounted.current)
      throw new Error(
        "Your account changed. Reopen this avatar before saving.",
      );
  };
  useEffect(() => {
    mounted.current = true;
    return () => {
      mounted.current = false;
    };
  }, []);
  useEffect(() => {
    if (!baseline && detail.data) setBaseline(detail.data.agent);
  }, [baseline, detail.data]);
  const current = useMemo(
    () =>
      baseline
        ? resolveAvatar({
            id: baseline.id,
            kind: baseline.kind,
            avatar: baseline.avatar ?? null,
          })
        : undefined,
    [baseline],
  );
  const [adopted, setAdopted] = useState(false);
  const [transparent, setTransparent] = useState(false);
  const avatar = draft ?? current;
  const unsupported =
    !!baseline?.avatar && !isValidAvatar(baseline.avatar) && !adopted;
  const state =
    baseline && avatar ? avatarEditorState(baseline, avatar) : undefined;
  const editable = enabled && !!baseline && canEditAvatar(baseline);
  const dirty = !done && (!!state?.dirty || adopted);
  const canSave =
    editable && !unsupported && dirty && isConnected && !busy && !exporting;

  const choose = (next: AvatarConfig) => {
    if (lock.current) return;
    setError(undefined);
    setDraft(next);
  };

  async function save() {
    if (lock.current || !canSave || !baseline || !avatar) return;
    lock.current = true;
    setBusy(true);
    setError(undefined);
    try {
      editableAvatarSchema.parse(avatar);
      const latest = (await agentsApi.detail(agentId)).agent;
      if (!mounted.current) return;
      guard();
      if (!canEditAvatar(latest))
        throw new Error("This agent's avatar can no longer be changed here.");
      if (avatarChangedElsewhere(baseline, latest))
        throw new Error(
          "This avatar changed elsewhere. Go back and review it before saving.",
        );
      await agentsApi.updateAvatar(
        agentId,
        avatarEditorState(baseline, avatar).patch,
        guard,
      );
      guard();
      await queryClient.invalidateQueries({ queryKey: ROOT });
      if (mounted.current) setDone(true);
    } catch (cause) {
      if (mounted.current)
        setError(
          cause instanceof ZodError
            ? "Choose every avatar part from the list"
            : errorMessage(cause),
        );
    } finally {
      lock.current = false;
      if (mounted.current) setBusy(false);
    }
  }
  async function exportSaved() {
    if (exportLock.current || lock.current || !current || !baseline) return;
    exportLock.current = true;
    setExporting(true);
    setError(undefined);
    try {
      const bytes = await pngAvatarAsync(
        current,
        transparent,
        mode,
        baseline.kind,
        { check: guard },
      );
      guard();
      const file = new File(Paths.cache, `nyxbot-face-v1-${agentId}.png`);
      file.write(bytes);
      guard();
      await shareAsync(file.uri, { mimeType: "image/png" });
    } catch (cause) {
      if (mounted.current && getAuthEpoch() === epoch.current)
        setError(errorMessage(cause));
    } finally {
      exportLock.current = false;
      if (mounted.current) setExporting(false);
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
  const options = useMemo(
    () => ({ title: t("Avatar"), headerRight }),
    [t, headerRight],
  );
  useWorkspaceBack({
    active: false,
    defaultRole: "back",
    onBack: navigation.goBack,
    disabled: busy,
    options,
    discardOnClose: dirty
      ? {
          title: t("Discard changes?"),
          message: t("Your unsaved avatar change will be lost."),
        }
      : undefined,
  });
  useEffect(() => {
    if (done && !busy) navigation.goBack();
  }, [done, busy, navigation]);

  const previewGroup = useRef({ y: 0 }),
    avatarBounds = useRef({ y: 0, height: 0 }),
    bannerBounds = useRef({ y: 0, height: 0 }),
    viewport = useRef({ offset: 0, height: 100000 });
  const [previewVisible, setPreviewVisible] = useState({
    avatar: true,
    banner: true,
  });
  const refreshVisibility = () => {
    const inView = (bounds: { y: number; height: number }) =>
      !bounds.height ||
      (previewGroup.current.y + bounds.y + bounds.height >
        viewport.current.offset &&
        previewGroup.current.y + bounds.y <
          viewport.current.offset + viewport.current.height);
    const next = {
      avatar: inView(avatarBounds.current),
      banner: inView(bannerBounds.current),
    };
    setPreviewVisible((previous) =>
      previous.avatar === next.avatar && previous.banner === next.banner
        ? previous
        : next,
    );
  };
  const locked = !editable || busy || done || exporting;
  return (
    <SettingsPage
      testID="avatar-editor-scroll"
      onScroll={(event) => {
        viewport.current = {
          offset: event.nativeEvent.contentOffset.y,
          height: event.nativeEvent.layoutMeasurement.height,
        };
        refreshVisibility();
      }}
      onLayout={(event) => {
        viewport.current.height = event.nativeEvent.layout.height;
        refreshVisibility();
      }}
    >
      {!isConnected && (
        <Notice message={t("You're offline. Connect to save changes.")} />
      )}
      {detail.isPending && (
        <SkeletonRows
          rows={3}
          icon={false}
          title
          accessibilityLabel={t("Loading agent")}
        />
      )}
      {detail.isError && !baseline && (
        <Notice tone="error" message={t(errorMessage(detail.error))} />
      )}
      {baseline && !editable && (
        <Notice
          message={t("This agent's avatar can no longer be changed here.")}
        />
      )}
      {error && <Notice tone="error" message={t(error)} />}
      {baseline && avatar && state ? (
        <>
          <SettingsGroup
            onLayout={(event) => {
              previewGroup.current.y = event.nativeEvent.layout.y;
              refreshVisibility();
            }}
            inset={false}
            footer={t(
              "How this agent looks in chats, groups and lists. Its expression follows what it is doing; the status text stays as it is.",
            )}
          >
            <View style={styles.preview}>
              <View
                testID="avatar-editor-live-preview"
                onLayout={(event) => {
                  avatarBounds.current = event.nativeEvent.layout;
                  refreshVisibility();
                }}
              >
                <AgentAvatar
                  agent={{ ...baseline, avatar }}
                  pixels={112}
                  mood={preview}
                  eventKey={`preview:${previewEvent}`}
                  animationVisible={previewVisible.avatar}
                  animated
                  label={t("Avatar preview")}
                />
              </View>
              <View
                testID="avatar-editor-voice-preview"
                style={{ width: "100%" }}
                onLayout={(event) => {
                  bannerBounds.current = event.nativeEvent.layout;
                  refreshVisibility();
                }}
              >
                <VoiceRippleBanner
                  visible={previewVisible.banner}
                  config={avatar}
                  kind={baseline.kind}
                  name={baseline.display_name?.trim() || baseline.name}
                  status={t("Preview expression · {{expression}}", { expression: t(avatarMoodLabel(preview)) })}
                  inputAccepting={preview === "listening"}
                  outputPlaying={preview === "speaking"}
                  outputLevel={previewLevel}
                />
              </View>
              {drawAvatar(avatar, { size: 112, mood: preview }).fallback ===
              "eye-grid" ? (
                <Text>
                  {t(
                    "A crisp face preserves these eyes at this texture's grid size.",
                  )}
                </Text>
              ) : null}
              <MoodPreview
                value={preview}
                onChange={(m) => {
                  setPreview(m);
                  setPreviewEvent((n) => n + 1);
                }}
              />
              <Text>
                {t("Native textures are static. Reactions use quiet motion.")}
              </Text>
              <View
                style={{
                  flexDirection: "row",
                  gap: 12,
                  alignItems: "flex-end",
                }}
              >
                {[16, 24, 44].map((p) => (
                  <View key={p}>
                    <AvatarStill
                      config={avatar}
                      kind={baseline.kind}
                      theme={mode}
                      pixels={p}
                    />
                    <Text>{t("{{size}} px", { size: p })}</Text>
                  </View>
                ))}
              </View>
              {unsupported ? (
                <>
                  <Text>
                    {t(
                      "This appearance uses a different version. Adopt the face kit or reset explicitly to change it.",
                    )}
                  </Text>
                  <Pressable
                    accessibilityRole="button"
                    onPress={() => setAdopted(true)}
                  >
                    <Text>{t("Adopt face kit")}</Text>
                  </Pressable>
                </>
              ) : null}
            </View>
          </SettingsGroup>
          <SettingsGroup
            inset
            footer={t("Changes apply everywhere this agent appears.")}
          >
            <SettingsRow
              icon={Dice5}
              tone="action"
              label={t("Randomise")}
              chevron={false}
              disabled={locked || unsupported}
              onPress={() => {
                choose(randomAvatar(Math.random, baseline.kind));
                setPreview("choosing");
                setPreviewEvent((n) => n + 1);
              }}
            />
            <SettingsRow
              icon={RotateCcw}
              tone="action"
              label={t("Reset to default")}
              chevron={false}
              disabled={locked || (state.atDefault && !unsupported)}
              onPress={() => {
                setAdopted(true);
                choose(state.fallback);
              }}
            />
          </SettingsGroup>
          {!unsupported
            ? AVATAR_EDITOR_SLOTS.map(({ field, label, group }) => (
                <SettingsGroup
                  key={field}
                  title={t("{{group}} · {{label}}", { group: t(group), label: t(label) })}
                  inset={false}
                  footer={t(
                    avatarChoiceLabel(field, avatar.choices[field] ?? ""),
                  )}
                >
                  <ChoiceGrid
                    field={field}
                    kind={baseline.kind}
                    label={t(label)}
                    avatar={avatar}
                    theme={mode}
                    disabled={locked}
                    onChoose={(id) => choose(chooseAvatar(avatar, field, id))}
                  />
                </SettingsGroup>
              ))
            : null}
          <SettingsGroup inset>
            <SettingsRow
              label={t("Transparent background")}
              disabled={busy || exporting}
              value={transparent ? t("On") : t("Off")}
              onPress={() => setTransparent(!transparent)}
            />
            <SettingsRow
              label={t("Export saved avatar")}
              value={exporting ? t("Preparing…") : "PNG 512px"}
              disabled={dirty || unsupported || busy || exporting}
              onPress={() => void exportSaved()}
            />
          </SettingsGroup>
        </>
      ) : null}
    </SettingsPage>
  );
}

/** Preview only: shows each expression; never saved. */
function MoodPreview({
  value,
  onChange,
}: {
  value: AvatarMood;
  onChange: (mood: AvatarMood) => void;
}) {
  const { t } = useAppLanguage();
  const { colors } = useTheme();
  return (
    <View
      accessibilityRole="radiogroup"
      accessibilityLabel={t("Preview expression")}
      style={styles.moods}
    >
      {AVATAR_MOODS.map((mood) => {
        const checked = mood === value;
        return (
          <Pressable
            key={mood}
            accessibilityRole="radio"
            accessibilityState={{ checked, selected: checked }}
            accessibilityLabel={t(avatarMoodLabel(mood))}
            onPress={() => onChange(mood)}
            hitSlop={4}
            style={[
              styles.mood,
              {
                borderColor: checked ? colors.primary : colors.borderSoft,
                backgroundColor: checked ? colors.navActive : "transparent",
              },
            ]}
          >
            <Text
              maxFontSizeMultiplier={1.6}
              style={[
                styles.moodText,
                { color: checked ? colors.textPrimary : colors.textSecondary },
              ]}
            >
              {t(avatarMoodLabel(mood))}
            </Text>
          </Pressable>
        );
      })}
    </View>
  );
}

function ChoiceGrid({
  field,
  label,
  avatar,
  kind,
  theme,
  disabled,
  onChoose,
}: {
  field: AvatarField;
  label: string;
  avatar: AvatarConfig;
  kind: string;
  theme: AvatarTheme;
  disabled: boolean;
  onChoose: (id: string) => void;
}) {
  const { t } = useAppLanguage(),
    { colors } = useTheme();
  const [expanded, setExpanded] = useState(false);
  const all = avatarChoices(field, kind),
    selected = avatar.choices[field],
    ids = expanded
      ? all
      : [
          ...new Set([
            ...all.slice(0, 7),
            ...(selected && all.includes(selected) ? [selected] : []),
          ]),
        ];
  const reason = all
    .map((id) => incompatibilityMessage(avatar, field, id))
    .find(Boolean);
  const reaction = field.startsWith("react_")
    ? reactionFor(avatar, field.slice(6) as AvatarMood, 44, true)
    : null;
  return (
    <View>
      <View
        accessibilityRole="radiogroup"
        accessibilityLabel={label}
        style={styles.grid}
      >
        {ids.map((id) => {
          const checked = selected === id,
            option = chooseAvatar(avatar, field, id),
            blocked = disabled || !isValidAvatar(option);
          return (
            <Pressable
              key={id}
              accessibilityRole="radio"
              accessibilityState={{
                checked,
                selected: checked,
                disabled: blocked,
              }}
              accessibilityLabel={t(avatarChoiceLabel(field, id))}
              accessibilityHint={
                incompatibilityMessage(avatar, field, id)
                  ? t(incompatibilityMessage(avatar, field, id)!)
                  : undefined
              }
              disabled={blocked}
              onPress={() => onChoose(id)}
              style={[
                styles.choice,
                {
                  borderColor: checked ? colors.primary : colors.borderSoft,
                  opacity: blocked ? 0.4 : 1,
                },
              ]}
            >
              {field === "ripple_palette" ? (
                <View
                  style={{
                    width: 64,
                    height: 44,
                    backgroundColor: FACE_GEOMETRY.ripples[id]!.bg,
                    flexDirection: "row",
                    flexWrap: "wrap",
                    gap: 4,
                    padding: 6,
                  }}
                >
                  {Array.from({ length: 8 }, (_, i) => (
                    <View
                      key={i}
                      style={{
                        width: 8,
                        height: 8,
                        borderRadius: 4,
                        backgroundColor:
                          FACE_GEOMETRY.ripples[id]![i % 2 ? "a" : "b"],
                      }}
                    />
                  ))}
                </View>
              ) : !field.startsWith("react_") && field !== "motion" ? (
                <View style={{ width: 64, height: 64 }}>
                  <View
                    style={{
                      position: "absolute",
                      width: 112,
                      height: 112,
                      left: -24,
                      top: -24,
                      transform: [{ scale: 64 / 112 }],
                    }}
                  >
                    <AvatarStill
                      config={option}
                      kind={kind}
                      theme={theme}
                      pixels={112}
                    />
                  </View>
                </View>
              ) : null}
              <Text style={{ color: colors.textPrimary }}>
                {t(avatarChoiceLabel(field, id))}
              </Text>
            </Pressable>
          );
        })}
      </View>
      {reason ? <Text>{t(reason)}</Text> : null}
      {all.length > 7 ? (
        <Pressable
          accessibilityRole="button"
          onPress={() => setExpanded(!expanded)}
        >
          <Text>
            {t(expanded ? "Show curated choices" : "Show full catalogue")}
          </Text>
        </Pressable>
      ) : null}
      {reaction ? (
        <Text>
          {t("Compact previews")}: {t(reactionLabel(reaction.effective))}.{" "}
          {t("Native textures are static. Reactions use quiet motion.")}
        </Text>
      ) : null}
    </View>
  );
}

const styles = StyleSheet.create({
  choice: {
    minWidth: 72,
    minHeight: 44,
    maxWidth: 160,
    borderRadius: 12,
    borderWidth: 1,
    padding: 8,
    gap: 4,
    alignItems: "center",
    overflow: "hidden",
  },
  preview: {
    alignItems: "center",
    gap: 14,
    paddingVertical: 18,
    paddingHorizontal: 12,
  },
  moods: {
    flexDirection: "row",
    flexWrap: "wrap",
    justifyContent: "center",
    gap: 6,
  },
  mood: {
    minHeight: 32,
    justifyContent: "center",
    borderWidth: 1,
    borderRadius: 999,
    paddingHorizontal: 10,
    paddingVertical: 4,
  },
  moodText: { fontFamily: fonts.body, fontSize: 13 },
  grid: { flexDirection: "row", flexWrap: "wrap", gap: 8, padding: 12 },
  partTile: {
    width: 52,
    height: 52,
    borderRadius: 12,
    borderWidth: 1.5,
    alignItems: "center",
    justifyContent: "center",
  },
  swatchTile: {
    width: 44,
    height: 44,
    borderRadius: 22,
    borderWidth: 2,
    alignItems: "center",
    justifyContent: "center",
  },
  swatch: {
    width: 32,
    height: 32,
    borderRadius: 16,
    borderWidth: StyleSheet.hairlineWidth,
  },
});
