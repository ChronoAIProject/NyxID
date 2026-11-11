import { fireEvent, render, screen } from "@testing-library/react-native";
import { NotificationBell } from "./NotificationBell";

jest.mock("../../theme/ThemeContext", () => {
  const { lightColors } = jest.requireActual("../../theme/mobileTheme");
  return { useTheme: () => ({ colors: lightColors, mode: "light" }) };
});

describe("NotificationBell", () => {
  it.each([[9, "9"], [10, "10"], [99, "99"], [100, "99+"]])("announces %s unread, displays %s and reports presses", async (unread, badge) => {
    const onPress = jest.fn();
    await render(<NotificationBell unread={Number(unread)} needsDecision={1} onPress={onPress} />);
    const bell = screen.getByRole("button", {
      name: `Notifications, ${unread} unread, 1 needs your decision`,
    });
    expect(screen.getByText(String(badge))).toBeTruthy();
    expect(screen.getByTestId("notification-bell-badge")).toHaveStyle({ position: "absolute", top: -6, right: -8 });
    expect(screen.getByTestId("notification-bell-glyph")).toHaveStyle({ position: "relative" });
    expect(screen.getByTestId("notification-bell-attention")).toBeTruthy();
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
