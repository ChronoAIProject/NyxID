import { memo, type ReactNode } from "react";
import { View } from "react-native";
import { AssistantMarkdownBody } from "./AssistantMarkdownBody";

/** One rhythm for saved, streaming and crew replies, without trailing space. */
export const AssistantReplyBody = memo(function AssistantReplyBody({
  text,
  activity,
  onLinkPress,
}: {
  text: string;
  activity?: ReactNode;
  onLinkPress?: (url: string) => boolean;
}) {
  return (
    <View style={{ gap: 16 }}>
      {activity ? <View style={{ gap: 8 }}>{activity}</View> : null}
      {text ? (
        <AssistantMarkdownBody text={text} onLinkPress={onLinkPress} />
      ) : null}
    </View>
  );
});
