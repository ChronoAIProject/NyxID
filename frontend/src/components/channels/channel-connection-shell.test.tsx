import { render } from "@testing-library/react";
import { expect, it } from "vitest";
import { useApplyTheme } from "@/hooks/use-theme";
import { useThemeStore } from "@/stores/theme-store";
import { ChannelConnectionShell } from "./channel-connection-shell";

function App({ open }: { open: boolean }) {
  useApplyTheme("assistant");
  return open ? (
    <ChannelConnectionShell platform="telegram-new" complete={false} embedded>
      Setup
    </ChannelConnectionShell>
  ) : (
    <p>Chat</p>
  );
}

it("preserves the parent app's theme, text size, density and motion after embedded setup closes", () => {
  useThemeStore.setState({
    mode: "dark",
    textScale: 1.25,
    density: 1.1,
    motion: "reduce",
  });
  const view = render(<App open={false} />);
  const before = {
    classes: document.documentElement.className,
    style: document.documentElement.getAttribute("style"),
  };
  expect(before.classes).toContain("theme-dark");
  view.rerender(<App open />);
  view.rerender(<App open={false} />);
  expect({
    classes: document.documentElement.className,
    style: document.documentElement.getAttribute("style"),
  }).toEqual(before);
});
