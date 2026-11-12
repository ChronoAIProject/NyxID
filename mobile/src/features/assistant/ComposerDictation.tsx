import { useAppLanguage } from "../../i18n/LanguageProvider";
import { useEffect, useRef, useState, type ReactNode } from "react";
import {
  ActivityIndicator,
  Keyboard,
  Pressable,
  ScrollView,
  StyleSheet,
  Text,
  useWindowDimensions,
  View,
} from "react-native";
import { Check, Mic, Square, X } from "lucide-react-native";
import { ActionButton, Label } from "../../components/workspace/WorkspaceUI";
import { useTheme } from "../../theme/ThemeContext";
import { fonts } from "../../theme/designTokens";
import { formatElapsed } from "./voice/dictationLevel";
import type { ComposerDictationState } from "./useComposerDictation";

import { DictationWaveform } from "./DictationWaveform";
import { composerInputMinHeight } from "./composerGeometry";
import { ChatComposerFrame } from "./ChatComposer";
import { useComposerHeight } from "../display/ChatLayoutContext";

export function ComposerDictationFrame({
  dictation,
  children,
}: {
  dictation: ComposerDictationState;
  children: ReactNode;
}) {
  return (
    <>
      <ComposerDictationNotice dictation={dictation} />
      <ChatComposerFrame bounce={!dictation.active}>
        {children}
      </ChatComposerFrame>
    </>
  );
}

export function ComposerDictationButton({
  dictation,
  disabled,
}: {
  dictation: ComposerDictationState;
  disabled?: boolean;
}) {
  const { t } = useAppLanguage();
  const { colors } = useTheme();
  const held = useRef(false);
  const busy =
    dictation.phase === "permission" ||
    dictation.phase === "connecting" ||
    dictation.phase === "stopping";
  return (
    <Pressable
      accessibilityRole="button"
      accessibilityLabel={
        dictation.recording ? t("Stop dictation") : t("Dictate message")
      }
      accessibilityHint={t(
        "Writes what you say into your message. Nothing is sent until you tap Send.",
      )}
      accessibilityState={{ disabled: !!disabled || busy, busy }}
      disabled={disabled || busy}
      onLongPress={() => {
        if (!dictation.active) {
          held.current = true;
          Keyboard.dismiss();
          void dictation.start();
        }
      }}
      onPressOut={() => {
        if (held.current) {
          held.current = false;
          dictation.stop();
        }
      }}
      onPress={() => {
        if (dictation.recording) dictation.stop();
        else {
          // Dictation runs in this composer; iOS asks for microphone and
          // speech recognition access itself the first time.
          Keyboard.dismiss();
          void dictation.start();
        }
      }}
      style={({ pressed }) => [
        styles.target,
        { opacity: disabled || busy ? 0.4 : pressed ? 0.65 : 1 },
      ]}
    >
      {busy ? (
        <ActivityIndicator size="small" color={colors.textSecondary} />
      ) : dictation.recording ? (
        <Square size={17} color={colors.danger} fill={colors.danger} />
      ) : (
        <Mic size={21} color={colors.textPrimary} />
      )}
    </Pressable>
  );
}

/** Dictated words grow like typed text, above the retained bottom toolbar. */
export function ComposerDictationReview({
  dictation,
  draft = "",
}: {
  dictation: ComposerDictationState;
  draft?: string;
}) {
  if (dictation.active)
    return <DictationInline dictation={dictation} draft={draft} />;
  return null;
}

/** Render beside the composer frame so errors never resize its input or toolbar. */
export function ComposerDictationNotice({
  dictation,
}: {
  dictation: ComposerDictationState;
}) {
  const { t } = useAppLanguage();
  if (!dictation.error) return null;
  return (
    <View style={styles.notice}>
      <Label accessibilityLiveRegion="polite" muted style={styles.noticeText}>
        {dictation.error}
      </Label>
      <View style={styles.noticeActions}>
        {dictation.permissionDenied && (
          <ActionButton
            label={t("Open Settings")}
            compact
            variant="secondary"
            onPress={dictation.openSettings}
          />
        )}
        <ActionButton
          label={t("Dismiss")}
          accessibilityLabel={t("Dismiss dictation message")}
          compact
          variant="secondary"
          onPress={dictation.discard}
        />
      </View>
    </View>
  );
}

function DictationInline({
  dictation,
  draft,
}: {
  dictation: ComposerDictationState;
  draft: string;
}) {
  const { t } = useAppLanguage();
  const { colors } = useTheme();
  const { fontScale } = useWindowDimensions();
  const minHeight = composerInputMinHeight(fontScale);
  const maxHeight = Math.max(minHeight, useComposerHeight());
  const scroller = useRef<ScrollView>(null);
  const { phase } = dictation;
  const [startedAt, setStartedAt] = useState<number>();
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    if (phase !== "listening") return;
    setStartedAt((value) => value ?? Date.now());
    setNow(Date.now());
    const timer = setInterval(() => setNow(Date.now()), 500);
    return () => clearInterval(timer);
  }, [phase]);
  const capturing = phase === "listening" || phase === "stopping";
  const before = draft.trim() ? draft.replace(/\s+$/, "") : "";
  const spoken = !!dictation.text;
  const status =
    phase === "permission"
      ? "Waiting for microphone…"
      : phase === "connecting"
        ? "Starting…"
        : undefined;
  return (
    <View testID="dictation-inline">
      <ScrollView
        ref={scroller}
        style={[styles.inlineText, { minHeight, maxHeight }]}
        contentContainerStyle={styles.inlineTextContent}
        // The newest words stay in view while speaking.
        onContentSizeChange={() =>
          scroller.current?.scrollToEnd?.({ animated: true })
        }
      >
        <Text
          accessibilityLiveRegion="polite"
          style={[styles.inlineBody, { color: colors.textPrimary }]}
        >
          {!before && !spoken ? (
            <Text style={{ color: colors.textTertiary }}>
              {t("Speak to dictate…")}
            </Text>
          ) : null}
          {before ? (
            <Text style={{ color: colors.textSecondary }}>
              {before}
              {spoken ? " " : ""}
            </Text>
          ) : null}
          {dictation.final}
          {dictation.interim ? (
            <Text style={{ color: colors.textTertiary }}>
              {dictation.final ? " " : ""}
              {dictation.interim}
            </Text>
          ) : null}
        </Text>
      </ScrollView>
      <View style={styles.inlineRow}>
        <Pressable
          accessibilityRole="button"
          accessibilityLabel={t("Discard dictation")}
          onPress={dictation.discard}
          style={styles.target}
        >
          {({ pressed }) => (
            <View
              style={[
                styles.inlineCircle,
                {
                  borderColor: colors.controlBorder,
                  opacity: pressed ? 0.6 : 1,
                },
              ]}
            >
              <X size={17} color={colors.textPrimary} />
            </View>
          )}
        </Pressable>
        <View style={styles.inlineWaveRow}>
          <View style={styles.inlineMeter}>
            {capturing ? (
              <DictationWaveform
                subscribe={dictation.subscribeLevel}
                height={22}
              />
            ) : (
              <View style={styles.inlineStatus}>
                <ActivityIndicator size="small" color={colors.textSecondary} />
                <Text
                  numberOfLines={1}
                  accessibilityLiveRegion="polite"
                  style={[styles.status, { color: colors.textSecondary }]}
                >
                  {status}
                </Text>
              </View>
            )}
          </View>
          <Text
            accessibilityLabel={`Recorded ${formatElapsed(startedAt === undefined ? 0 : now - startedAt)}`}
            style={[styles.elapsed, { color: colors.textTertiary }]}
          >
            {formatElapsed(startedAt === undefined ? 0 : now - startedAt)}
          </Text>
        </View>
        <Pressable
          accessibilityRole="button"
          accessibilityLabel={t("Finish dictation")}
          accessibilityHint={t(
            "Adds the words to your message. Nothing is sent until you tap Send.",
          )}
          accessibilityState={{
            disabled: phase !== "listening",
            busy: phase === "stopping",
          }}
          disabled={phase !== "listening"}
          onPress={dictation.stop}
          style={styles.target}
        >
          {({ pressed }) => (
            <View
              style={[
                styles.inlineCircle,
                styles.inlineFinish,
                {
                  backgroundColor: colors.textPrimary,
                  opacity:
                    phase !== "listening" && phase !== "stopping"
                      ? 0.28
                      : pressed
                        ? 0.7
                        : 1,
                },
              ]}
            >
              {phase === "stopping" ? (
                <ActivityIndicator size="small" color={colors.bg} />
              ) : (
                <Check size={18} color={colors.bg} strokeWidth={2.5} />
              )}
            </View>
          )}
        </Pressable>
      </View>
    </View>
  );
}

const styles = StyleSheet.create({
  target: {
    width: 44,
    height: 44,
    alignItems: "center",
    justifyContent: "center",
  },
  notice: { paddingHorizontal: 12, paddingTop: 8, gap: 6 },
  inlineText: { flexGrow: 0, flexShrink: 1 },
  inlineTextContent: {
    paddingHorizontal: 12,
    paddingTop: 12,
    paddingBottom: 10,
  },
  inlineBody: { fontFamily: fonts.bodyRegular, fontSize: 14, lineHeight: 20 },
  inlineWaveRow: {
    flex: 1,
    minWidth: 0,
    flexDirection: "row",
    alignItems: "center",
    minHeight: 44,
    paddingHorizontal: 8,
    gap: 8,
  },
  inlineRow: { flexDirection: "row", alignItems: "center", height: 44 },
  inlineMeter: { flex: 1, minWidth: 0, justifyContent: "center" },
  inlineStatus: { flexDirection: "row", alignItems: "center", gap: 8 },
  inlineCircle: {
    width: 34,
    height: 34,
    borderRadius: 17,
    borderWidth: StyleSheet.hairlineWidth,
    alignItems: "center",
    justifyContent: "center",
  },
  inlineFinish: { borderWidth: 0 },
  noticeText: { fontSize: 13 },
  noticeActions: { flexDirection: "row", flexWrap: "wrap", gap: 8 },
  status: { fontFamily: fonts.body, fontSize: 13, lineHeight: 18 },
  elapsed: { fontFamily: fonts.mono, fontSize: 12, lineHeight: 18 },
});
