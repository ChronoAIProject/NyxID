import { fireEvent, render, screen } from "@testing-library/react-native";
import { Pressable, Text } from "react-native";
import AsyncStorage from "@react-native-async-storage/async-storage";
import { LanguageProvider, useAppLanguage } from "../i18n/LanguageProvider";
import { LANGUAGE_STORAGE_KEY } from "../i18n/language";
import { AppErrorBoundary } from "./AppErrorBoundary";
import { crashLifecycle } from "../lib/crashReporting/runtime";

// Crash reporting talks to native code and the network; the boundary only
// hands it the render error.
// The saved language decides; the device list reads a native module.
jest.mock("../i18n/deviceLanguages", () => ({
  deviceLanguages: () => ["en-US"],
}));
jest.mock("../lib/crashReporting/runtime", () => ({
  crashLifecycle: { capture: jest.fn(async () => undefined) },
}));

it("the mounted error fallback follows app language changes", async () => {
  await AsyncStorage.setItem(LANGUAGE_STORAGE_KEY, "en");
  const report = jest.spyOn(console, "error").mockImplementation(() => {});
  function BrokenPage(): never { throw new Error("test render failure"); }
  function LanguageControl() {
    const { choose } = useAppLanguage();
    return <Pressable onPress={() => choose("zh-CN")} accessibilityRole="button" accessibilityLabel="Change language"><Text>简体中文</Text></Pressable>;
  }
  try {
    await render(<LanguageProvider>
      <LanguageControl />
      <AppErrorBoundary><BrokenPage /></AppErrorBoundary>
    </LanguageProvider>);
    expect(await screen.findByText("Something went wrong")).toBeTruthy();
    expect(crashLifecycle!.capture).toHaveBeenCalledWith(
      expect.any(Error),
      "render_error",
    );
    await fireEvent.press(screen.getByRole("button", { name: "Change language" }));
    expect(screen.queryByText("Something went wrong")).toBeNull();
    expect(screen.getByText("出了问题")).toBeTruthy();
  } finally {
    report.mockRestore();
  }
});
