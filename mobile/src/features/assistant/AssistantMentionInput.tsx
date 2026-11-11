import { AgentAvatar } from "../../components/AgentAvatar";
import type { AgentIdentity } from "../../components/agentAvatarModel";
import { useComposerHeight } from "../display/ChatLayoutContext";
import {
  ScrollEdgeFades,
  useScrollEdges,
} from "../../components/workspace/ScrollEdgeFades";
import { useEffect, useRef, useState, type ReactNode } from "react";
import {
  Platform,
  Pressable,
  ScrollView,
  StyleSheet,
  Text,
  TextInput,
  View,
  type StyleProp,
  type TextStyle,
} from "react-native";
import { Clipboard, FileText, Image } from "lucide-react-native";
import { useTheme } from "../../theme/ThemeContext";
import { fonts } from "../../theme/designTokens";
import {
  activeMention,
  insertMention,
  mentionCandidates,
  type NativeTextSelection,
} from "./assistantMentions";

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

export function AssistantMentionInput({
  value,
  onChangeText,
  mentions = [],
  editable,
  placeholder,
  accessibilityLabel,
  generation = 0,
  style,
  renderActions,
}: {
  value: string;
  onChangeText: (value: string) => void;
  mentions?: readonly AssistantMentionOption[];
  editable: boolean;
  placeholder: string;
  accessibilityLabel: string;
  generation?: number | string;
  style?: StyleProp<TextStyle>;
  /** Put composer controls below the text so multiline drafts use its full width. */
  renderActions?: (mentionAction: ReactNode) => ReactNode;
}) {
  const { colors } = useTheme();
  const maxHeight = useComposerHeight();
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
    44,
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
      accessibilityLabel="Mention an attachment or agent"
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
                    accessibilityLabel={`Mention ${option.label}, @${option.handle}`}
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
                No matching mentions
              </Text>
            )}
          </ScrollView>
        </View>
      ) : null}
      <View style={[styles.inputFrame, renderActions && styles.fullWidthInput]}>
        <TextInput
          key={revision}
          ref={input}
          accessibilityLabel={accessibilityLabel}
          value={value}
          editable={editable}
          placeholder={placeholder}
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
            if (current()) setFocused(true);
          }}
          onBlur={() => {
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
          onLayout={(event) => overflow.layout(event.nativeEvent.layout.height)}
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
            { color: colors.textPrimary },
            style,
            // iOS sizes a multiline input to its text; a measured height pins it at one line.
            Platform.OS === "ios"
              ? { maxHeight }
              : { height: inputHeight, maxHeight },
          ]}
        />
        <ScrollEdgeFades edges={overflow.edges} color={colors.composerBg} />
      </View>
      {renderActions ? renderActions(mentionAction) : mentionAction}
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
    paddingTop: 12,
    paddingBottom: 10,
    textAlignVertical: "top",
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
