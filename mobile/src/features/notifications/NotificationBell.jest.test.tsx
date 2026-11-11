import { fireEvent, render, screen } from "@testing-library/react-native";
import { NotificationBell } from "./NotificationBell";

jest.mock("../../theme/ThemeContext", () => {
  const { lightColors } = jest.requireActual("../../theme/mobileTheme");
  return { useTheme: () => ({ colors: lightColors, mode: "light" }) };
});

describe("NotificationBell", () => {
  it.each([[9, "9"], [10, "10"], [24, "24"], [99, "99"], [100, "99+"]])("announces %s unread, displays %s and reports presses", async (unread, badge) => {
    const onPress = jest.fn();
    await render(<NotificationBell unread={Number(unread)} needsDecision={1} onPress={onPress} />);
    const bell = screen.getByRole("button", {
      name: `Notifications, ${unread} unread, 1 needs your decision`,
    });
    expect(screen.getByText(String(badge))).toBeTruthy();
    expect(screen.getByTestId("notification-bell-badge-ring")).toHaveStyle({ position: "absolute", left: 8.5, top: -8, borderWidth: 2 });
    expect(screen.getByTestId("notification-bell-badge")).toHaveStyle({ height: 14, minWidth: 14, paddingHorizontal: 3 });
    expect(screen.getByText(String(badge))).toHaveStyle({ fontSize: 10, lineHeight: 10, fontWeight: "600" });
    expect(screen.getByTestId("notification-bell-glyph")).toHaveStyle({ position: "relative" });
    expect(screen.queryByTestId("notification-bell-attention")).toBeNull();
    const { lightColors } = jest.requireActual("../../theme/mobileTheme");
    expect(screen.getByTestId("notification-bell-badge")).toHaveStyle({ backgroundColor: lightColors.warning });
    await fireEvent.press(bell);
    expect(onPress).toHaveBeenCalledTimes(1);
  });

  it("hides the badge when nothing is unread and respects disabled", async () => {
    const onPress = jest.fn();
    await render(<NotificationBell unread={0} needsDecision={0} onPress={onPress} disabled />);
    const bell = screen.getByRole("button", { name: "Notifications" });
    expect(screen.queryByTestId("notification-bell-badge")).toBeNull();
    await fireEvent.press(bell);
    expect(onPress).not.toHaveBeenCalled();
  });
});


it("shows one decision-count pill at zero unread, and a neutral pill without decisions", async () => {
  const { lightColors } = jest.requireActual("../../theme/mobileTheme");
  const view = await render(<NotificationBell unread={0} needsDecision={2} onPress={() => {}} />);
  expect(screen.getByRole("button", { name: "Notifications, 0 unread, 2 need your decision" })).toBeTruthy();
  expect(screen.getByText("2")).toBeTruthy();
  expect(screen.getByTestId("notification-bell-badge")).toHaveStyle({ backgroundColor: lightColors.warning });
  await view.rerender(<NotificationBell unread={10} needsDecision={0} onPress={() => {}} />);
  expect(screen.getByText("10")).toBeTruthy();
  expect(screen.getByTestId("notification-bell-badge")).toHaveStyle({ backgroundColor: lightColors.primary });
});
