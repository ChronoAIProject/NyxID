import { useState } from "react";
import { Dimensions, Text } from "react-native";
import { fireEvent, render, screen } from "@testing-library/react-native";
import { ChatComposerInput, ChatComposerToolbar } from "./ChatComposerInput";
import { ComposerSubmitButton } from "./ChatComposer";
import { composerInputMinHeight } from "./composerGeometry";
import type { ComposerDictationState } from "./useComposerDictation";

jest.mock("../../i18n/LanguageProvider", () => ({ useAppLanguage: () => ({ t: (text: string) => text }) }));

jest.mock("../../theme/ThemeContext", () => {
  const { lightColors } = jest.requireActual("../../theme/mobileTheme");
  return { useTheme: () => ({ colors: lightColors, mode: "light" }) };
});
jest.mock("../display/ChatLayoutContext", () => ({
  useComposerHeight: () => 120,
}));
jest.mock("../../components/workspace/ScrollEdgeFades", () => ({
  ScrollEdgeFades: () => null,
  useScrollEdges: () => ({ edges: {}, layout() {}, content() {}, scroll() {} }),
}));
jest.mock("./ComposerDictation", () => {
  const React = jest.requireActual("react");
  const { Pressable } = jest.requireActual("react-native");
  return {
    ComposerDictationReview: () => null,
    ComposerDictationButton: () => React.createElement(Pressable, {
      accessibilityRole: "button",
      accessibilityLabel: "Dictate message",
      style: { width: 44, height: 44 },
    }),
  };
});

const dictation = { active: false } as ComposerDictationState;
const singleLineHeight = composerInputMinHeight(Dimensions.get("window").fontScale);
function Composer({ initial = "", crew = false }) {
  const [draft, setDraft] = useState(initial);
  return (
    <ChatComposerInput
      value={draft}
      onChangeText={setDraft}
      dictation={dictation}
      editable
      placeholder="Message NyxBot"
      accessibilityLabel="Message"
      expandedContent={crew ? <Text>Crew recipients</Text> : undefined}
      mentions={crew ? [{ id: "a", handle: "agent", label: "Agent", kind: "agent" }] : []}
      renderActions={(mention, layout) => (
        <ChatComposerToolbar
          layout={layout}
          mention={mention}
          attach={{ label: "Attach", onPress: () => {} }}
          dictation={dictation}
          dictationDisabled={false}
        >
          <ComposerSubmitButton kind="send" label="Send" onPress={() => setDraft("")} />
        </ChatComposerToolbar>
      )}
    />
  );
}

describe("idle and active composer layout", () => {
  it.each([false, true])("collapses an idle composer and preserves its native input through focus, typing, clearing and blur (crew=%s)", async (crew) => {
    await render(<Composer crew={crew} />);
    const input = screen.getByLabelText("Message");
    const controls = () => screen.getByTestId("chat-composer-toolbar");
    expect(controls()).toHaveStyle({ position: "absolute" });
    expect(screen.getByTestId("chat-composer-input-frame")).toHaveStyle({ paddingLeft: 48, paddingRight: 92 });
    expect(input).toHaveStyle({ height: singleLineHeight, maxHeight: singleLineHeight });
    expect(screen.queryByText("Crew recipients")).toBeNull();
    expect(screen.queryByRole("button", { name: "Mention an attachment or agent" })).toBeNull();
    expect(screen.getByRole("button", { name: "Attach" })).toBeTruthy();
    expect(screen.getByRole("button", { name: "Dictate message" })).toBeTruthy();
    expect(screen.getByRole("button", { name: "Send" })).toBeTruthy();

    await fireEvent(input, "focus");
    expect(controls()).not.toHaveStyle({ position: "absolute" });
    if (crew) {
      expect(screen.getByText("Crew recipients")).toBeTruthy();
      expect(screen.getByRole("button", { name: "Mention an attachment or agent" })).toBeTruthy();
    }
    await fireEvent.changeText(input, "  A draft\nwith two lines  ");
    await fireEvent(input, "blur");
    expect(controls()).not.toHaveStyle({ position: "absolute" });
    expect(screen.getByLabelText("Message")).toBe(input);

    await fireEvent(input, "focus");
    await fireEvent.changeText(input, "");
    expect(controls()).not.toHaveStyle({ position: "absolute" });
    expect(screen.getByLabelText("Message")).toBe(input);
    await fireEvent(input, "blur");
    expect(controls()).toHaveStyle({ position: "absolute" });
    expect(input).toHaveStyle({ height: singleLineHeight, maxHeight: singleLineHeight });
    expect(screen.queryByText("Crew recipients")).toBeNull();
  });

  it.each(["", " ", "\n\n", " \t\n "])("keeps an unfocused whitespace-only draft %j to one line", async (initial) => {
    await render(<Composer initial={initial} />);
    const input = screen.getByLabelText("Message");
    await fireEvent(input, "contentSizeChange", { nativeEvent: { contentSize: { height: 200 } } });
    expect(screen.getByTestId("chat-composer-toolbar")).toHaveStyle({ position: "absolute" });
    expect(input).toHaveStyle({ height: singleLineHeight, maxHeight: singleLineHeight });
    await fireEvent(input, "focus");
    expect(screen.getByTestId("chat-composer-toolbar")).not.toHaveStyle({ position: "absolute" });
    await fireEvent(input, "blur");
    expect(screen.getByTestId("chat-composer-toolbar")).toHaveStyle({ position: "absolute" });
  });

  it("opens a restored text draft expanded and collapses after sending it while unfocused", async () => {
    await render(<Composer initial="  Restored message  " />);
    expect(screen.getByTestId("chat-composer-toolbar")).not.toHaveStyle({ position: "absolute" });
    await fireEvent.press(screen.getByRole("button", { name: "Send" }));
    expect(screen.getByTestId("chat-composer-toolbar")).toHaveStyle({ position: "absolute" });
    expect(screen.getByLabelText("Message")).toHaveDisplayValue("");
  });
});
