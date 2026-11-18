import assert from "node:assert/strict";
import test from "node:test";
import { URL, fileURLToPath } from "node:url";
import { deferred, hookHarness } from "../features/assistant/assistantHookTestHarness";
import * as appearance from "./chatAppearance";
import * as theme from "./mobileTheme";

test("bubble preference hydrates before children mount and rapid changes persist in order", async () => {
  const stored = deferred<string | null>();
  const firstWrite = deferred<void>();
  const writes: string[] = [];
  const barStyles: string[] = [];
  let systemScheme = "light";
  const harness = hookHarness<any>({
    "react/jsx-runtime": { jsx: (type: unknown, props: unknown) => ({ type, props }) },
    "react-native": { Platform: { OS: "android" }, useColorScheme: () => systemScheme },
    "expo-navigation-bar": { setButtonStyleAsync: async (style: string) => { barStyles.push(style); } },
    "./chatAppearance": appearance,
    "./mobileTheme": theme,
    "expo-secure-store": {
      getItemAsync: (key: string) => key.includes("bubble") ? stored.promise : Promise.resolve("light"),
      setItemAsync: async (_key: string, value: string) => {
        writes.push(value);
        if (writes.length === 1) await firstWrite.promise;
      },
    },
  });
  const { ThemeProvider } = harness.load(fileURLToPath(new URL("./ThemeContext.tsx", import.meta.url))) as {
    ThemeProvider: (props: any) => any;
  };
  harness.mount(() => ThemeProvider({ children: "chat" }));
  assert.equal(harness.value(), null);
  stored.resolve("lavender");
  await harness.settle();
  assert.equal(harness.value().props.value.bubblePalette, "lavender");
  harness.value().props.value.setBubblePalette("graphite");
  harness.flush();
  harness.value().props.value.setBubblePalette("gray");
  await harness.settle();
  assert.equal(harness.value().props.value.bubblePalette, "gray");
  assert.deepEqual(writes, ["graphite"]);
  firstWrite.resolve();
  await harness.settle();
  assert.deepEqual(writes, ["graphite", "gray"]);
  assert.deepEqual(barStyles, ["dark"]);
  harness.value().props.value.setPreference("dark");
  await harness.settle();
  harness.value().props.value.setPreference("system");
  await harness.settle();
  assert.deepEqual(barStyles, ["dark", "light", "dark"]);
  systemScheme = "dark";
  harness.render();
  assert.deepEqual(barStyles, ["dark", "light", "dark", "light"]);
  harness.unmount();
});
