import { useRef } from "react";
import { findNodeHandle, View } from "react-native";
import { ActionButton } from "../../components/workspace/WorkspaceUI";

/** A hosted connect link from a reply, shown as a real action instead of inline link text. */
export function AssistantConnectButton({
  label,
  onPress,
}: {
  label: string;
  onPress: (trigger?: number) => void;
}) {
  const anchor = useRef<View>(null);
  return (
    <View ref={anchor} style={{ marginTop: 10, alignSelf: "stretch" }}>
      <ActionButton
        label={label}
        onPress={() => {
          const handle = anchor.current ? findNodeHandle(anchor.current) : null;
          onPress(handle ?? undefined);
        }}
      />
    </View>
  );
}
