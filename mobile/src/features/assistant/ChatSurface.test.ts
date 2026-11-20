import assert from "node:assert/strict";
import test from "node:test";
import { URL, fileURLToPath } from "node:url";
import { hookHarness } from "./assistantHookTestHarness";
import {
  chatBubbleColors,
  userBubbleCorners,
} from "../../theme/chatAppearance";

function surface(mode: "light" | "dark", bubblePalette = "gray") {
  const column = { width: "100%", maxWidth: 758, alignSelf: "center" };
  const h = hookHarness<any>({
    "react/jsx-runtime": {
      jsx: (type: any, props: any) => ({ type, props }),
      jsxs: (type: any, props: any) => ({ type, props }),
    },
    "react-native": {
      View: "View",
      Pressable: "Pressable",
      StyleSheet: { create: (s: any) => s },
    },
    "../../theme/ThemeContext": {
      useTheme: () => ({
        mode,
        bubblePalette,
        colors: { composerBg: "#fff", composerBorder: "#ccc", shadowColor: "#000" },
      }),
    },
    "../display/ChatLayoutContext": { useChatColumn: () => column },
  });
  const ui = h.load(
    fileURLToPath(new URL("./ChatSurface.tsx", import.meta.url)),
  ) as any;
  return { h, ui, column };
}

for (const mode of ["light", "dark"] as const) {
  test(`${mode}: private and crew user turns share bubble geometry and external footers`, () => {
    const { h, ui } = surface(mode);
    const s = ui.chatSurfaceStyles;
    for (const author of [undefined, "Crew author / recipients"]) {
      h.mount(() =>
        ui.ChatMessageTurn({
          user: true,
          notes: "History note",
          author,
          children: "Message",
          footer: "Copy / date",
        }),
      );
      const turn = h.value();
      assert.equal(turn.props.style, s.userTurn);
      assert.equal(turn.props.children[0], "History note");
      const row = turn.props.children[1];
      assert.equal(row.props.style, s.userRow);
      assert.equal(row.props.style.marginLeft, 38);
      assert.equal(row.props.style.paddingVertical, 6);
      assert.equal(row.props.style.gap, 2);
      assert.equal(row.props.children[0], author);
      const bubble = row.props.children[1];
      const style = Object.assign({}, ...bubble.props.style);
      assert.equal(
        style.backgroundColor,
        chatBubbleColors(mode, "gray").background,
      );
      assert.equal(style.paddingHorizontal, 14);
      assert.equal(style.paddingVertical, 10);
      assert.equal(style.gap, 8);
      for (const [key, value] of Object.entries(userBubbleCorners))
        assert.equal(style[key], value);
      assert.equal(bubble.props.children, "Message");
      assert.equal(row.props.children[2], "Copy / date");
      h.unmount();
    }
  });
  test(`${mode}: private and crew agent turns and text/voice docks use the same grid`, () => {
    const { h, ui, column } = surface(mode);
    const s = ui.chatSurfaceStyles;
    for (const author of [undefined, "Agent avatar / author"]) {
      h.mount(() =>
        ui.ChatMessageTurn({
          author,
          children: "Reply / activity",
          footer: "Copy / Report / date",
        }),
      );
      assert.equal(h.value().props.style, s.message);
      assert.equal(h.value().props.style.paddingHorizontal, undefined);
      assert.equal(h.value().props.style.paddingVertical, 10);
      assert.equal(h.value().props.style.gap, 8);
      assert.deepEqual(
        [...h.value().props.children],
        [undefined, author, "Reply / activity", "Copy / Report / date"],
      );
      h.unmount();
    }
    for (const children of [
      "Private text",
      "Crew text + recipients",
      "Private voice",
      "Crew voice",
    ]) {
      h.mount(() => ui.ChatComposerDock({ children }));
      assert.equal(h.value().props.style.paddingHorizontal, 12);
      const dock = h.value().props.children;
      assert.equal(dock.props.style[0], s.composer);
      assert.equal(dock.props.style[1], column);
      assert.equal(dock.props.children, children);
      assert.equal(dock.props.style[0].paddingTop, undefined);
      h.unmount();
    }
    assert.equal(s.transcriptHeader.marginBottom, 20);
    assert.equal(ui.ChatTranscriptSeparator().props.style.height, 20);
    assert.equal(s.footer.paddingTop, 20);
    assert.equal(s.footer.gap, 16);
    assert.equal(s.messageText.fontSize, 14);
    assert.equal(s.messageText.lineHeight, 23);
    assert.equal(s.messageLabel.fontSize, 11);
    assert.equal(s.row.gap, 8);
    assert.equal(s.chatPage.paddingBottom, 4);
    assert.equal(s.chatPage.gap, 8);
  });
}

test("plain chat controls (sheet Close) keep a 44pt target and pressed/disabled feedback", () => {
  const { ui } = surface("dark");
  let presses = 0;
  for (const disabled of [false, true]) {
    const button = ui.ChatIconButton({
      label: "Close",
      disabled,
      onPress: () => presses++,
      children: "Icon",
    });
    const flat = (pressed: boolean) =>
      Object.assign({}, ...button.props.style({ pressed }).filter(Boolean));
    assert.equal(flat(false).minWidth, 44);
    assert.equal(flat(false).minHeight, 44);
    assert.equal(flat(true).opacity, disabled ? 0.4 : 0.65);
    assert.equal(button.props.disabled, disabled);
    assert.equal(button.props.accessibilityState.disabled, disabled);
    if (!disabled) button.props.onPress();
  }
  assert.equal(presses, 1);
});

test("header bars float a chat control as the shared glass button in every layout", () => {
  for (const mode of ["light", "dark"] as const) {
    const { ui } = surface(mode);
    for (const disabled of [false, true]) {
      const button = ui.ChatIconButton({
        label: "Chat options",
        floating: true,
        disabled,
        onPress() {},
        children: "Icon",
      });
      // The one glass primitive: same glass, 44pt capsule, highlight and
      // disabled treatment as the rail and the identity pill.
      assert.equal(button.type, "FloatingButton");
      assert.equal(button.props.accessibilityLabel, "Chat options");
      assert.equal(button.props.disabled, disabled);
      assert.equal(button.props.children, "Icon");
      assert.equal(button.props.style, undefined, "no styling of its own");
      assert.equal(button.props.capsuleStyle, undefined);
    }
  }
});
