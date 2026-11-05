import { RangeControl } from "@/components/ui/range-control";
import {
  CHAT_CONTENT_WIDTH,
  COMPOSER_MAX_HEIGHT,
  useThemeStore,
} from "@/stores/theme-store";

export function ChatWidthControl() {
  const width = useThemeStore((state) => state.chatContentWidth);
  const setWidth = useThemeStore((state) => state.setChatContentWidth);
  return (
    <RangeControl
      label="Chat content width"
      description="Maximum width for messages and the composer. Smaller screens fit automatically."
      saved={width}
      onChange={setWidth}
      {...CHAT_CONTENT_WIDTH}
      defaultValue={CHAT_CONTENT_WIDTH.default}
      formatValue={(value) => `${value}px`}
    />
  );
}

export function ComposerHeightControl() {
  const height = useThemeStore((state) => state.composerMaxHeight);
  const setHeight = useThemeStore((state) => state.setComposerMaxHeight);
  return (
    <RangeControl
      label="Maximum composer height"
      description="How tall your draft can grow before it scrolls. Short drafts stay compact and always fit the screen."
      saved={height}
      onChange={setHeight}
      {...COMPOSER_MAX_HEIGHT}
      defaultValue={COMPOSER_MAX_HEIGHT.default}
      formatValue={(value) => `${value}px`}
    />
  );
}
