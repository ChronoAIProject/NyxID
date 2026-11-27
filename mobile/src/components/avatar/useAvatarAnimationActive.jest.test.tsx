import { act, render, screen } from "@testing-library/react-native";
import { Text, AppState, type AppStateStatus } from "react-native";
import { NavigationContext } from "@react-navigation/native";
import { useAvatarAnimationActive } from "./useAvatarAnimationActive";

function Probe() {
  return <Text>{useAvatarAnimationActive() ? "running" : "resting"}</Text>;
}

function fakeNavigation(focused: boolean) {
  const listeners = new Map<string, () => void>();
  return {
    isFocused: () => focused,
    addListener: (event: string, listener: () => void) => {
      listeners.set(event, listener);
      return () => listeners.delete(event);
    },
    emit: (event: "focus" | "blur") => listeners.get(event)?.(),
  };
}

let appStateListener: ((state: AppStateStatus) => void) | undefined;
beforeEach(() => {
  Object.defineProperty(AppState, "currentState", {
    value: "active",
    configurable: true,
  });
  jest.spyOn(AppState, "addEventListener").mockImplementation((_, listener) => {
    appStateListener = listener as (state: AppStateStatus) => void;
    return { remove: () => (appStateListener = undefined) } as never;
  });
});
afterEach(() => jest.restoreAllMocks());

test("animated avatars rest while their screen is unfocused", async () => {
  const navigation = fakeNavigation(true);
  await render(
    <NavigationContext.Provider value={navigation as never}>
      <Probe />
    </NavigationContext.Provider>,
  );
  expect(screen.getByText("running")).toBeTruthy();
  await act(async () => navigation.emit("blur"));
  expect(screen.getByText("resting")).toBeTruthy();
  await act(async () => navigation.emit("focus"));
  expect(screen.getByText("running")).toBeTruthy();
});

test("animated avatars rest while the app is not active, and outside a navigator only the app state counts", async () => {
  await render(<Probe />);
  expect(screen.getByText("running")).toBeTruthy();
  await act(async () => appStateListener?.("background"));
  expect(screen.getByText("resting")).toBeTruthy();
  await act(async () => appStateListener?.("active"));
  expect(screen.getByText("running")).toBeTruthy();
});
