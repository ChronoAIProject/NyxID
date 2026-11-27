import { useAppLanguage } from "../../i18n/LanguageProvider";
import { translateApp } from "../../i18n/appTranslation";
import { composerInputMinHeight } from "./composerGeometry";
import { AgentAvatar } from "../../components/AgentAvatar";
import type { AgentIdentity } from "../../components/agentAvatarModel";
import { useComposerHeight } from "../display/ChatLayoutContext";
import {
  ScrollEdgeFades,
  useScrollEdges,
} from "../../components/workspace/ScrollEdgeFades";
import { useEffect, useRef, useState, type ReactNode } from "react";
import {
  Text,
  Platform,
  Pressable,
  ScrollView,
  StyleSheet,
  TextInput,
  useWindowDimensions,
  View,
  type StyleProp,
  type TextStyle,
} from "react-native";
import { Clipboard, FileText, Image } from "lucide-react-native";
import { useTheme } from "../../theme/ThemeContext";
import { fonts } from "../../theme/designTokens";
import { chatComposerFocus } from "./chatComposerFocus";
import { composerCompact } from "./composerCompact";
import {
  activeMention,
  insertMention,
  mentionCandidates,
  type NativeTextSelection,
} from "./assistantMentions";
import { useRailScrollIndicator } from "../../components/workspace/workspaceFloatingInset";

export type AssistantMentionOption = {
  id: string;
  handle: string;
  label: string;
  kind: "agent" | "image" | "file" | "pasted-text";
  detail?: string;
  agent?: AgentIdentity;
};

type ScopedSelection = {
  revision: number;
  selection: NativeTextSelection;
};

/** Handed to `renderActions`: whether the idle composer is one row, and where
 * its leading/trailing controls end so the input's text clears them. */
export type ComposerActionsLayout = {
  compact: boolean;
  onLeadingWidth: (width: number) => void;
  onTrailingWidth: (width: number) => void;
};

/** One row only while nothing could be hidden: unfocused and empty. */

// Space between a one-row composer's controls and its text.
const COMPACT_TEXT_GAP = 4;

export function AssistantMentionInput({
  value,
  onChangeText,
  mentions = [],
  editable,
  onInputLayout,
  placeholder,
  accessibilityLabel,
  generation = 0,
  style,
  expandedContent,
  renderActions,
  compactWhenIdle = true,
}: {
  value: string;
  onChangeText: (value: string) => void;
  mentions?: readonly AssistantMentionOption[];
  editable: boolean;
  onInputLayout?: (event: import("react-native").LayoutChangeEvent, currentInput: () => TextInput | null) => void;
  placeholder: string;
  accessibilityLabel: string;
  generation?: number | string;
  style?: StyleProp<TextStyle>;
  expandedContent?: ReactNode;
  /** Put composer controls below the text so multiline drafts use its full width. */
  renderActions?: (
    mentionAction: ReactNode,
    layout: ComposerActionsLayout,
  ) => ReactNode;
  /** An unfocused composer with only whitespace collapses to one row on every window. */
  compactWhenIdle?: boolean;
}) {
  const railIndicator = useRailScrollIndicator();
  useAppLanguage();
  const { colors } = useTheme();
  const maxHeight = useComposerHeight();
  const { fontScale } = useWindowDimensions();
  const overflow = useScrollEdges();
  const input = useRef<TextInput>(null);
  const mounted = useRef(true);
  const end = { start: value.length, end: value.length };
  const scope = useRef({
    generation,
    value,
    revision: 0,
    emitted: undefined as string | undefined,
    selection: end,
  });
  // Remount only for externally replaced drafts. Native typing, including IME
  // updates and deleting the final character, keeps the same native input.
  if (
    scope.current.generation !== generation ||
    (scope.current.value !== value && scope.current.emitted !== value)
  ) {
    scope.current.revision += 1;
    scope.current.selection = end;
    scope.current.generation = generation;
  }
  if (scope.current.value !== value) {
    scope.current.value = value;
    scope.current.emitted = undefined;
  }
  const revision = scope.current.revision;
  const [contentSize, setContentSize] = useState({ revision, height: 44 });
  const inputHeight = Math.max(
    composerInputMinHeight(fontScale),
    Math.min(
      maxHeight,
      contentSize.revision === revision ? contentSize.height : 44,
    ),
  );
  const [cursor, setCursor] = useState<ScopedSelection>({
    revision,
    selection: end,
  });
  const [forcedCursor, setForcedCursor] = useState<ScopedSelection | null>(
    null,
  );
  const [focused, setFocused] = useState(false);
  const compact =
    !!renderActions && composerCompact(compactWhenIdle, focused, value);
  // An empty draft, or a collapsed whitespace-only one, shows the one-line
  // prompt; the whitespace stays in the draft.
  const showPlaceholder = !value || compact;
  // Measured from the toolbar in either layout; 44pt targets until then.
  const [leadingWidth, setLeadingWidth] = useState(44);
  const [trailingWidth, setTrailingWidth] = useState(88);
  const actionsLayout: ComposerActionsLayout = {
    compact,
    onLeadingWidth: (width) => {
      if (mounted.current && width > 0) setLeadingWidth(Math.ceil(width));
    },
    onTrailingWidth: (width) => {
      if (mounted.current && width > 0) setTrailingWidth(Math.ceil(width));
    },
  };
  const textInsets = compact
    ? {
        paddingLeft: leadingWidth + COMPACT_TEXT_GAP,
        paddingRight: trailingWidth + COMPACT_TEXT_GAP,
      }
    : undefined;
  const selection =
    cursor.revision === revision ? cursor.selection : scope.current.selection;
  const trigger = editable && focused ? activeMention(value, selection) : null;
  const candidates = trigger ? mentionCandidates(mentions, trigger.query) : [];
  const live = useRef({ editable, onChangeText });
  live.current = { editable, onChangeText };
  const current = () =>
    mounted.current &&
    scope.current.revision === revision &&
    live.current.editable;

  useEffect(() => {
    mounted.current = true;
    return () => {
      mounted.current = false;
    };
  }, []);

  function write(text: string, nextSelection: NativeTextSelection) {
    if (!current()) return;
    scope.current.emitted = text;
    scope.current.selection = nextSelection;
    setCursor({ revision, selection: nextSelection });
    setForcedCursor({ revision, selection: nextSelection });
    live.current.onChangeText(text);
    input.current?.focus();
  }

  function openMentions() {
    if (!current() || !mentions.length) return;
    if (activeMention(scope.current.value, scope.current.selection)) {
      input.current?.focus();
      setFocused(true);
      return;
    }
    const caret = Math.min(
      Math.max(0, scope.current.selection.end),
      scope.current.value.length,
    );
    const before = scope.current.value.slice(0, caret);
    const after = scope.current.value.slice(caret);
    const prefix = before && !/\s$/u.test(before) ? " @" : "@";
    // A separator after @ preserves the existing suffix when a candidate is
    // chosen, including when the button is used in the middle of a word.
    const separator = /^\s/u.test(after) ? "" : " ";
    const text = `${before}${prefix}${separator}${after}`;
    const position = before.length + prefix.length;
    setFocused(true);
    write(text, { start: position, end: position });
  }

  const mentionAction = mentions.length ? (
    <Pressable
      accessibilityRole="button"
      accessibilityLabel={translateApp("Mention an attachment or agent")}
      accessibilityState={{ disabled: !editable, expanded: !!trigger }}
      disabled={!editable}
      onPress={openMentions}
      style={styles.mentionButton}
    >
      <Text
        style={[
          styles.at,
          { color: editable ? colors.primaryOnTint : colors.textMuted },
        ]}
      >
        @
      </Text>
    </Pressable>
  ) : null;

  return (
    <View style={[styles.container, renderActions && styles.fullWidth]}>
      {trigger && mentions.length ? (
        <View
          style={[
            styles.popup,
            { backgroundColor: colors.card, borderColor: colors.border },
          ]}
        >
          <ScrollView
            keyboardShouldPersistTaps="handled"
            nestedScrollEnabled
            style={styles.options}
            {...railIndicator}
          >
            {candidates.length ? (
              candidates.map((option) => {
                const Icon =
                  option.kind === "image"
                    ? Image
                    : option.kind === "pasted-text"
                      ? Clipboard
                      : FileText;
                return (
                  <Pressable
                    key={option.id}
                    accessibilityRole="button"
                    accessibilityLabel={translateApp(
                      "Mention {{name}}, @{{handle}}",
                      { name: option.label, handle: option.handle },
                    )}
                    disabled={!editable}
                    onPress={() => {
                      if (!current()) return;
                      const active = activeMention(
                        scope.current.value,
                        scope.current.selection,
                      );
                      if (!active) return;
                      const next = insertMention(
                        scope.current.value,
                        active,
                        option.handle,
                      );
                      write(next.text, next.selection);
                    }}
                    style={({ pressed }) => [
                      styles.option,
                      {
                        backgroundColor: pressed
                          ? colors.navActive
                          : "transparent",
                      },
                    ]}
                  >
                    {option.kind === "agent" && option.agent ? (
                      <AgentAvatar agent={option.agent} size="md" />
                    ) : (
                      <Icon size={18} color={colors.primaryOnTint} />
                    )}
                    <View style={styles.optionText}>
                      <Text
                        numberOfLines={1}
                        style={[styles.label, { color: colors.textPrimary }]}
                      >
                        {option.label}
                      </Text>
                      <Text
                        numberOfLines={1}
                        style={[styles.detail, { color: colors.textSecondary }]}
                      >
                        @{option.handle}
                        {option.detail ? ` · ${option.detail}` : ""}
                      </Text>
                    </View>
                  </Pressable>
                );
              })
            ) : (
              <Text style={[styles.noMatches, { color: colors.textSecondary }]}>
                {translateApp("No matching mentions")}
              </Text>
            )}
          </ScrollView>
        </View>
      ) : null}
      {!compact ? expandedContent : null}
      <View
        testID="chat-composer-input-frame"
        style={[
          styles.inputFrame,
          renderActions && styles.fullWidthInput,
          textInsets,
        ]}
      >
        <TextInput
          key={revision}
          ref={input}
          accessibilityLabel={accessibilityLabel}
          value={value}
          editable={editable}
          placeholder={Platform.OS === "ios" ? "" : placeholder}
          accessibilityHint={
            Platform.OS === "ios" && showPlaceholder ? placeholder : undefined
          }
          placeholderTextColor={colors.textMuted}
          selectionColor={colors.primary}
          multiline
          scrollEnabled
          maxLength={32768}
          autoFocus={focused && editable}
          selection={
            forcedCursor?.revision === revision
              ? forcedCursor.selection
              : undefined
          }
          onFocus={() => {
            if (input.current) chatComposerFocus.focused(input.current);
            if (current()) setFocused(true);
          }}
          onBlur={() => {
            if (input.current) chatComposerFocus.blurred(input.current);
            if (mounted.current && scope.current.revision === revision)
              setFocused(false);
          }}
          onChangeText={(text) => {
            if (!current()) return;
            const previous = scope.current.value;
            const previousSelection = scope.current.selection;
            const position = Math.min(
              text.length,
              Math.max(
                0,
                previousSelection.start +
                  text.length -
                  (previous.length -
                    (previousSelection.end - previousSelection.start)),
              ),
            );
            const nextSelection = { start: position, end: position };
            scope.current.emitted = text;
            scope.current.selection = nextSelection;
            setCursor({ revision, selection: nextSelection });
            setForcedCursor(null);
            live.current.onChangeText(text);
          }}
          onSelectionChange={({ nativeEvent }) => {
            if (!current()) return;
            const nextSelection = nativeEvent.selection;
            scope.current.selection = nextSelection;
            setCursor({ revision, selection: nextSelection });
            if (
              forcedCursor?.revision === revision &&
              forcedCursor.selection.start === nextSelection.start &&
              forcedCursor.selection.end === nextSelection.end
            )
              setForcedCursor(null);
          }}
          onLayout={(event) => {
            overflow.layout(event.nativeEvent.layout.height);
            onInputLayout?.(event, () => input.current);
          }}
          onContentSizeChange={(event) => {
            if (!mounted.current || scope.current.revision !== revision) return;
            const height = Math.ceil(event.nativeEvent.contentSize.height);
            if (!Number.isFinite(height) || height < 0) return;
            setContentSize((previous) =>
              previous.revision === revision && previous.height === height
                ? previous
                : { revision, height },
            );
            overflow.content(height);
          }}
          onScroll={(event) =>
            overflow.scroll(event.nativeEvent.contentOffset.y)
          }
          style={[
            styles.input,
            {
              color: colors.textPrimary,
              minHeight: composerInputMinHeight(fontScale),
            },
            style,
            // Idle whitespace stays to one line on both native platforms.
            compact
              ? { height: composerInputMinHeight(fontScale), maxHeight: composerInputMinHeight(fontScale) }
              : Platform.OS === "ios"
              ? {
                  height: showPlaceholder
                    ? composerInputMinHeight(fontScale)
                    : undefined,
                  maxHeight,
                }
              : { height: inputHeight, maxHeight },
          ]}
        />
        {Platform.OS === "ios" && showPlaceholder ? (
          // Remounted per font scale: a live text-size change must re-measure
          // this overlay, or it keeps its old line box and clips the glyphs.
          <Text
            pointerEvents="none"
            accessibilityElementsHidden
            importantForAccessibility="no"
            numberOfLines={1}
            style={[
              styles.emptyPlaceholder,
              { color: colors.textMuted },
              renderActions &&
                (textInsets
                  ? {
                      left: textInsets.paddingLeft,
                      right: textInsets.paddingRight,
                    }
                  : { left: 12, right: 12 }),
            ]}
          >
            {placeholder}
          </Text>
        ) : null}
        <ScrollEdgeFades edges={overflow.edges} color={colors.composerBg} />
      </View>
      {renderActions
        ? renderActions(mentionAction, actionsLayout)
        : mentionAction}
    </View>
  );
}

const styles = StyleSheet.create({
  container: {
    flex: 1,
    minWidth: 0,
    flexDirection: "row",
    alignItems: "flex-end",
    position: "relative",
  },
  inputFrame: { flex: 1, minWidth: 0 },
  fullWidth: { flex: 0, flexDirection: "column", alignItems: "stretch" },
  fullWidthInput: {
    flex: 0,
    paddingHorizontal: 12,
    borderTopLeftRadius: 22,
    borderTopRightRadius: 22,
    overflow: "hidden",
  },
  input: {
    minWidth: 0,
    minHeight: 44,
    maxHeight: 120,
    fontFamily: fonts.bodyRegular,
    fontSize: 14,
    lineHeight: 20,
    paddingTop: 12,
    paddingBottom: 10,
    textAlignVertical: "top",
  },
  emptyPlaceholder: {
    position: "absolute",
    top: 12,
    left: 0,
    right: 0,
    fontFamily: fonts.bodyRegular,
    fontSize: 14,
    lineHeight: 20,
  },
  mentionButton: {
    width: 44,
    height: 44,
    alignItems: "center",
    justifyContent: "center",
  },
  at: { fontFamily: fonts.bodySemi, fontSize: 22 },
  popup: {
    position: "absolute",
    left: 0,
    right: 0,
    bottom: "100%",
    marginBottom: 8,
    borderWidth: 1,
    borderRadius: 12,
    overflow: "hidden",
    zIndex: 20,
    elevation: 8,
  },
  options: { maxHeight: 180 },
  option: {
    minHeight: 52,
    flexDirection: "row",
    alignItems: "center",
    gap: 10,
    paddingHorizontal: 12,
    paddingVertical: 8,
  },
  optionText: { flex: 1, minWidth: 0, gap: 3 },
  label: { fontFamily: fonts.body, fontSize: 13 },
  detail: { fontFamily: fonts.bodyRegular, fontSize: 11 },
  noMatches: {
    fontFamily: fonts.bodyRegular,
    fontSize: 13,
    paddingHorizontal: 12,
    paddingVertical: 16,
  },
});
