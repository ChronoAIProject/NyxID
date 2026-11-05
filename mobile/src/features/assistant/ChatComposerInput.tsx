import type { ComponentProps, ReactNode } from "react";
import { View } from "react-native";
import { AssistantMentionInput } from "./AssistantMentionInput";
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
            ? { height: 0, overflow: "hidden", opacity: 0 }
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

/** Private and crew chats share the same control order and dictation affordance. */
export function ChatComposerToolbar({
  attach,
  mention,
  settings,
  dictation,
  dictationDisabled,
  children,
}: {
  attach: ComponentProps<typeof ComposerAttachButton>;
  mention: ReactNode;
  settings?: ReactNode;
  dictation: ComposerDictationState;
  dictationDisabled: boolean;
  children: ReactNode;
}) {
  return (
    <ChatComposerRow>
      <ComposerAttachButton {...attach} />
      {mention}
      {settings}
      <View style={{ flex: 1 }} />
      <ComposerDictationButton
        dictation={dictation}
        disabled={dictationDisabled}
      />
      {children}
    </ChatComposerRow>
  );
}
