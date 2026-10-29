import {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useRef,
  useState,
} from "react";
import { Keyboard, useWindowDimensions } from "react-native";
import AsyncStorage from "@react-native-async-storage/async-storage";
import {
  composerHeightLimit,
  defaultChatLayout,
  normalizeChatLayout,
  type ChatLayout,
} from "./chatLayout";

const STORAGE_KEY = "nyxid_chat_layout_v1";
const ChatLayoutContext = createContext({
  layout: defaultChatLayout,
  update: (_value: Partial<ChatLayout>) => {},
  reset: () => {},
});
export function ChatLayoutProvider({
  children,
}: {
  children: React.ReactNode;
}) {
  const [layout, setLayout] = useState(defaultChatLayout);
  const revision = useRef(0);
  const current = useRef(layout);
  const writes = useRef(Promise.resolve());
  const persist = useCallback((next: ChatLayout) => {
    current.current = next;
    setLayout(next);
    writes.current = writes.current
      .catch(() => {})
      .then(() => AsyncStorage.setItem(STORAGE_KEY, JSON.stringify(next)));
    void writes.current.catch(() => {});
  }, []);
  useEffect(() => {
    let active = true;
    const initialRevision = revision.current;
    void AsyncStorage.getItem(STORAGE_KEY)
      .then((stored) => {
        if (!active || revision.current !== initialRevision) return;
        let value: unknown;
        try {
          value = stored ? JSON.parse(stored) : null;
        } catch {
          value = null;
        }
        persist(normalizeChatLayout(value));
      })
      .catch(() => {});
    return () => {
      active = false;
    };
  }, [persist]);
  const update = useCallback(
    (value: Partial<ChatLayout>) => {
      revision.current++;
      persist(normalizeChatLayout({ ...current.current, ...value }));
    },
    [persist],
  );
  const reset = useCallback(() => {
    revision.current++;
    persist({ ...defaultChatLayout });
  }, [persist]);
  return (
    <ChatLayoutContext.Provider value={{ layout, update, reset }}>
      {children}
    </ChatLayoutContext.Provider>
  );
}
export function useChatLayout() {
  return useContext(ChatLayoutContext);
}
export function useChatColumn() {
  const { layout } = useChatLayout();
  return {
    width: "100%" as const,
    maxWidth: layout.columnWidth,
    alignSelf: "center" as const,
  };
}
export function useComposerHeight() {
  const { layout } = useChatLayout();
  const { height } = useWindowDimensions();
  const [keyboardTop, setKeyboardTop] = useState<number | null>(null);
  useEffect(() => {
    const show = Keyboard.addListener("keyboardDidShow", (event) =>
      setKeyboardTop(event.endCoordinates.screenY),
    );
    const hide = Keyboard.addListener("keyboardDidHide", () =>
      setKeyboardTop(null),
    );
    return () => {
      show.remove();
      hide.remove();
    };
  }, []);
  return composerHeightLimit(
    layout.composerMaxHeight,
    Math.min(height, keyboardTop ?? height),
  );
}
