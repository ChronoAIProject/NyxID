import type { ComponentProps, ReactNode } from "react";
import { StyleSheet, View } from "react-native";
import {
  AssistantMentionInput,
  type ComposerActionsLayout,
} from "./AssistantMentionInput";
import {
  ComposerDictationButton,
  ComposerDictationReview,
} from "./ComposerDictation";
import { ChatComposerRow, ComposerAttachButton } from "./ChatComposer";
import type { ComposerDictationState } from "./useComposerDictation";

type InputProps = ComponentProps<typeof AssistantMentionInput>;

/** Keep the input mounted so a held dictation button always receives its release. */
export function ChatComposerInput({
  dictation,
  ...input
}: InputProps & { dictation: ComposerDictationState }) {
  return (
    <>
      <ComposerDictationReview dictation={dictation} draft={input.value} />
      <View
        style={
          dictation.active
            ? {
                position: "absolute",
                height: 0,
                overflow: "hidden",
                opacity: 0,
              }
            : undefined
        }
        accessibilityElementsHidden={dictation.active}
        importantForAccessibility={
          dictation.active ? "no-hide-descendants" : "auto"
        }
      >
        <AssistantMentionInput {...input} />
      </View>
    </>
  );
}

/**
 * Private and crew chats share the same control order and dictation affordance.
 * The tree is identical in both layouts, so a held dictation button and every
 * control stay mounted when an idle composer collapses to one row; only the
 * mode chip is left out of that row.
 */
export function ChatComposerToolbar({
  attach,
  mention,
  settings,
  dictation,
  dictationDisabled,
  layout,
  children,
}: {
  attach: ComponentProps<typeof ComposerAttachButton>;
  mention: ReactNode;
  settings?: ReactNode;
  dictation: ComposerDictationState;
  dictationDisabled: boolean;
  /** From the input's `renderActions`; omitted keeps the two-row layout. */
  layout?: ComposerActionsLayout;
  children: ReactNode;
}) {
  const compact = !!layout?.compact;
  return (
    <ChatComposerRow overlay={compact}>
      <View
        testID="chat-composer-leading"
        style={styles.group}
        onLayout={(event) =>
          layout?.onLeadingWidth(event.nativeEvent.layout.width)
        }
      >
        <ComposerAttachButton {...attach} />
        {mention}
      </View>
      {compact ? null : settings}
      <View style={styles.spacer} pointerEvents="none" />
      <View
        testID="chat-composer-trailing"
        style={styles.group}
        onLayout={(event) =>
          layout?.onTrailingWidth(event.nativeEvent.layout.width)
        }
      >
        <ComposerDictationButton
          dictation={dictation}
          disabled={dictationDisabled}
        />
        {children}
      </View>
    </ChatComposerRow>
  );
}

const styles = StyleSheet.create({
  group: { flexDirection: "row", alignItems: "center" },
  spacer: { flex: 1 },
});
