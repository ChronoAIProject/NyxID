import { StyleSheet, View, useWindowDimensions } from "react-native";
import { useNavigation } from "@react-navigation/native";
import type { NativeStackNavigationProp } from "@react-navigation/native-stack";
import type { WorkspaceSheetParamList } from "../../app/navigationTypes";
import {
  SettingsGroup,
  SettingsRow,
} from "../../components/settings/SettingsList";
import { useChatLayout } from "./ChatLayoutContext";
import {
  chatLayoutTitle,
  defaultChatLayout,
  offersChatWidth,
} from "./chatLayout";
import { spacing } from "../../theme/designTokens";
import { ChatBubbleSettings } from "./ChatBubbleSettings";

/** Settings › Display groups for the message bubble and chat layout. */
export function ChatDisplaySettings() {
  const { width } = useWindowDimensions();
  const navigation =
    useNavigation<NativeStackNavigationProp<WorkspaceSheetParamList>>();
  const { layout, reset } = useChatLayout();
  const customised =
    layout.columnWidth !== defaultChatLayout.columnWidth ||
    layout.composerMaxHeight !== defaultChatLayout.composerMaxHeight;
  return (
    <View style={styles.groups}>
      <ChatBubbleSettings />
      <SettingsGroup
        title="Chat layout"
        inset={false}
        footer="Saved on this device for every conversation."
      >
        {offersChatWidth(width) ? (
          <SettingsRow
            label={chatLayoutTitle("columnWidth")}
            value={`${layout.columnWidth} pt`}
            onPress={() =>
              navigation.navigate("ChatLayoutEdit", { key: "columnWidth" })
            }
          />
        ) : null}
        <SettingsRow
          label={chatLayoutTitle("composerMaxHeight")}
          value={`${layout.composerMaxHeight} pt`}
          onPress={() =>
            navigation.navigate("ChatLayoutEdit", { key: "composerMaxHeight" })
          }
        />
        {customised ? (
          <SettingsRow centered label="Reset Chat Layout" onPress={reset} />
        ) : null}
      </SettingsGroup>
    </View>
  );
}

const styles = StyleSheet.create({
  groups: { gap: spacing.huge },
});
