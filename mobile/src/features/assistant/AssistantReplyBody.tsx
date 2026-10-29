import { memo, type ReactNode } from "react";
import { View } from "react-native";
import { AssistantMarkdownBody } from "./AssistantMarkdownBody";

/** One rhythm for saved, streaming and crew replies, without trailing space. */
export const AssistantReplyBody = memo(function AssistantReplyBody({
  text,
  activity,
  variant,
  onLinkPress,
}: {
  text: string;
  activity?: ReactNode;
  variant?: "private" | "crew";
  onLinkPress?: (url: string) => boolean;
}) {
  return (
    <View style={{ gap: 16 }}>
      {activity ? <View style={{ gap: 8 }}>{activity}</View> : null}
      {text ? (
        <AssistantMarkdownBody
          text={text}
          variant={variant}
          onLinkPress={onLinkPress}
        />
      ) : null}
    </View>
  );
});
