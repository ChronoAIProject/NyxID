import {
  fireEvent,
  render,
  screen,
  within,
} from "@testing-library/react-native";
import { DictationLanguagePicker } from "./DictationLanguagePicker";
import { languageOptions } from "./dictationLanguageLabels";

jest.mock("../../../theme/ThemeContext", () => {
  const { lightColors } = jest.requireActual("../../../theme/mobileTheme");
  return { useTheme: () => ({ colors: lightColors, mode: "light" }) };
});
jest.mock("../../../i18n/LanguageProvider", () => ({
  useAppLanguage: () => ({
    language: "en",
    t: (phrase: string, values?: { language: string }) =>
      values ? phrase.replace("{{language}}", values.language) : phrase,
  }),
}));
// The picker receives the settings state; importing its type must not load native recognition.
jest.mock("expo-speech-recognition", () => ({
  ExpoSpeechRecognitionModule: {},
}));
jest.mock("react-native-safe-area-context", () => {
  const actual = jest.requireActual("react-native-safe-area-context");
  const { View } = jest.requireActual("react-native");
  return {
    ...actual,
    SafeAreaProvider: ({ children, ...props }: React.PropsWithChildren) => (
      <View testID="modal-safe-area-provider">
        <actual.SafeAreaProvider
          initialMetrics={{
            frame: { x: 0, y: 0, width: 393, height: 852 },
            insets: { top: 59, right: 0, bottom: 34, left: 0 },
          }}
          {...props}
        >
          {children}
        </actual.SafeAreaProvider>
      </View>
    ),
  };
});

function language() {
  return {
    loading: false,
    listUnavailable: false,
    explanation: undefined,
    value: "ja-JP",
    resolved: "ja-JP",
    label: "Japanese (Japan)",
    summary: "Japanese (Japan)",
    suggested: ["zh-CN", "ja-JP"],
    options: [
      {
        value: "automatic",
        label: "Device language",
        detail: "Chinese (China)",
      },
      ...languageOptions(["zh-CN", "ja-JP", "en-US"]),
    ],
    choose: jest.fn(),
  };
}
it("wraps the full-screen modal content in its own safe-area provider on first open", async () => {
  await render(
    <DictationLanguagePicker language={language()} onClose={jest.fn()} />,
  );
  const content = within(screen.getByTestId("modal-safe-area-provider"));
  expect(
    content.getByRole("header", { name: "Dictation language" }),
  ).toBeTruthy();
  expect(content.getByLabelText("Close")).toBeTruthy();
  expect(content.getByLabelText("Search languages")).toBeTruthy();
  expect(content.getByText("All languages")).toBeTruthy();
});
it("keeps the active language visible while picking another language for the next recording", async () => {
  const state = language();
  const close = jest.fn();
  await render(
    <DictationLanguagePicker
      currentRecordingLanguage="Chinese (China)"
      language={state}
      onClose={close}
    />,
  );
  expect(
    screen.getByRole("header", { name: "Language for the next recording" }),
  ).toBeTruthy();
  const search = screen.getByLabelText("Search languages");
  await fireEvent.changeText(search, "en-US");
  expect(
    screen.getByText("This recording continues in Chinese (China)."),
  ).toBeTruthy();
  await fireEvent.press(screen.getByRole("radio"));
  expect(state.choose).toHaveBeenCalledWith("en-US");
  expect(close).toHaveBeenCalledTimes(1);
});
it("searches endonyms and fullwidth codes, retains checked selection, and applies one choice for the next recording", async () => {
  const state = language();
  const close = jest.fn();
  await render(<DictationLanguagePicker language={state} onClose={close} />);
  expect(screen.getByText("Suggested")).toBeTruthy();
  expect(screen.getByText("All languages")).toBeTruthy();
  expect(screen.getByText("Device language")).toBeTruthy();
  await fireEvent.changeText(
    screen.getByLabelText("Search languages"),
    "ＪＡ－ｊｐ",
  );
  const choices = screen.getAllByRole("radio");
  expect(choices).toHaveLength(2);
  expect(choices.every((row) => row.props.accessibilityState.checked)).toBe(
    true,
  );
  await fireEvent.press(choices[0]!);
  expect(state.choose).toHaveBeenCalledWith("ja-JP");
  expect(close).toHaveBeenCalledTimes(1);
});
it("shows an empty search state and can close without changing the language", async () => {
  const state = language();
  const close = jest.fn();
  await render(<DictationLanguagePicker language={state} onClose={close} />);
  await fireEvent.changeText(
    screen.getByLabelText("Search languages"),
    "no such language",
  );
  expect(screen.getByText("No languages found")).toBeTruthy();
  await fireEvent.press(screen.getByLabelText("Close"));
  expect(close).toHaveBeenCalledTimes(1);
  expect(state.choose).not.toHaveBeenCalled();
});
