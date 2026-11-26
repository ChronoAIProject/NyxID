import { fireEvent, render, screen } from "@testing-library/react-native";
import { PrimaryButton } from "./PrimaryButton";

jest.mock("../theme/ThemeContext", () => {
  const { lightColors } = jest.requireActual("../theme/mobileTheme");
  return { useTheme: () => ({ colors: lightColors }) };
});

for (const kind of ["primary", "ghost", "danger"] as const) {
  it(`${kind} PrimaryButton exposes its visible label, button role and live disabled state`, async () => {
    const press = jest.fn();
    const view = await render(
      <PrimaryButton
        kind={kind}
        label="Scan or enter another code"
        onPress={press}
      />,
    );
    const button = () =>
      screen.getByRole("button", { name: "Scan or enter another code" });
    expect(button().props.accessibilityState.disabled).toBe(false);
    await fireEvent.press(button());
    expect(press).toHaveBeenCalledTimes(1);
    await view.rerender(
      <PrimaryButton
        kind={kind}
        label="Scan or enter another code"
        onPress={press}
        disabled
      />,
    );
    expect(button().props.accessibilityState.disabled).toBe(true);
    await fireEvent.press(button());
    expect(press).toHaveBeenCalledTimes(1);
    expect(screen.getByText("Scan or enter another code")).toBeTruthy();
    await view.unmount();
  });
}
