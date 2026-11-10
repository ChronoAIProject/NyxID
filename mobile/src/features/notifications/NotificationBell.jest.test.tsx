import { fireEvent, render, screen } from "@testing-library/react-native";
import { NotificationBell } from "./NotificationBell";

jest.mock("../../theme/ThemeContext", () => {
  const { lightColors } = jest.requireActual("../../theme/mobileTheme");
  return { useTheme: () => ({ colors: lightColors, mode: "light" }) };
});

describe("NotificationBell", () => {
  it("announces its counts, caps the badge at 9+ and reports presses", async () => {
    const onPress = jest.fn();
    await render(<NotificationBell unread={15} needsDecision={1} onPress={onPress} />);
    const bell = screen.getByRole("button", {
      name: "Notifications, 15 unread, 1 needs your decision",
    });
    expect(screen.getByText("9+")).toBeTruthy();
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
