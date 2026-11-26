import {
  createContext,
  useContext,
  useEffect,
  useState,
  useMemo,
  type ReactNode,
  type Dispatch,
  type SetStateAction,
} from "react";
import { View } from "react-native";
import { RIGHT_PANEL_WIDTH } from "./workspaceLayout";

type PanelContext = {
  enabled: boolean;
  content: ReactNode;
  setRightPanel: Dispatch<SetStateAction<ReactNode>>;
};
export const RightPanelContext = createContext<PanelContext>({
  enabled: false,
  content: null,
  setRightPanel() {},
});
export function useRightPanel() {
  return useContext(RightPanelContext);
}

/** Pages opt in to the shared slot; ordinary pages allocate no panel width. */
export function RightPanelProvider({
  enabled,
  children,
}: {
  enabled: boolean;
  children: ReactNode;
}) {
  const [content, setRightPanel] = useState<ReactNode>(null);
  const value = useMemo(() => ({ enabled, content, setRightPanel }), [enabled, content]);
  return (
    <RightPanelContext.Provider value={value}>
      {children}
    </RightPanelContext.Provider>
  );
}
export function RightPanelRegistration({ children }: { children: ReactNode }) {
  const { enabled, setRightPanel } = useRightPanel();
  useEffect(() => {
    setRightPanel(enabled ? children : null);
  }, [enabled, children, setRightPanel]);
  useEffect(() => () => setRightPanel(null), [setRightPanel]);
  return null;
}
export function RightPanelSlot() {
  const { enabled, content } = useRightPanel();
  return enabled && content ? (
    <View
      testID="workspace-right-panel"
      style={{ width: RIGHT_PANEL_WIDTH, flexShrink: 0, minHeight: 0 }}
    >
      {content}
    </View>
  ) : null;
}
