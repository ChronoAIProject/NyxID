import { fireEvent, render, screen } from "@testing-library/react-native";
import {
  NYXBOT_PUSH_FOOTER_NO_DEVICE,
  NYXBOT_PUSH_FOOTER_READY,
  NyxBotPushSwitches,
} from "./NyxBotPushSwitches";
import { DEFAULT_NOTIFICATION_PUSH } from "../notifications/notificationContracts";

jest.mock("../../theme/ThemeContext", () => {
  const { lightColors } = jest.requireActual("../../theme/mobileTheme");
  return { useTheme: () => ({ colors: lightColors, mode: "light" }) };
});

const LABELS = [
  "Automation results",
  "Automation problems",
  "Decisions needed",
  "Agent updates",
  "Chat updates",
  "Group mentions",
];

describe("NyxBotPushSwitches", () => {
  it("renders one switch per kind and saves the complete six-key object on every toggle", async () => {
    const onSave = jest.fn();
    await render(
      <NyxBotPushSwitches
        preferences={{ ...DEFAULT_NOTIFICATION_PUSH, needs_decision: true }}
        saving={false}
        pushReady
        onSave={onSave}
      />,
    );
    expect(screen.getByText(NYXBOT_PUSH_FOOTER_READY)).toBeTruthy();
    const switches = LABELS.map((label) => screen.getByRole("switch", { name: label }));
    expect(switches.map((control) => control.props.value)).toEqual([false, false, true, false, false, false]);
    expect(switches.every((control) => control.props.disabled === false)).toBe(true);
    await fireEvent(switches[4]!, "valueChange", true);
    await fireEvent(switches[2]!, "valueChange", false);
    expect(onSave.mock.calls).toEqual([
      [
        {
          automation_result: false,
          automation_problem: false,
          needs_decision: true,
          agent_update: false,
          chat_update: true,
          group_mention: false,
        },
      ],
      [
        {
          automation_result: false,
          automation_problem: false,
          needs_decision: false,
          agent_update: false,
          chat_update: false,
          group_mention: false,
        },
      ],
    ]);
  });

  it("keeps every switch disabled until the saved values are known and while a save is in flight", async () => {
    const onSave = jest.fn();
    const loading = await render(
      <NyxBotPushSwitches preferences={undefined} saving={false} pushReady={false} onSave={onSave} />,
    );
    expect(screen.getByText(NYXBOT_PUSH_FOOTER_NO_DEVICE)).toBeTruthy();
    const pending = screen.getByRole("switch", { name: "Chat updates" });
    expect(pending.props.disabled).toBe(true);
    await fireEvent(pending, "valueChange", true);
    expect(onSave).not.toHaveBeenCalled();
    // Unmount settles asynchronously in RNTL 14; wait so act scopes do not overlap.
    await loading.unmount();
    await render(
      <NyxBotPushSwitches preferences={DEFAULT_NOTIFICATION_PUSH} saving pushReady onSave={onSave} />,
    );
    expect(LABELS.every((label) => screen.getByRole("switch", { name: label }).props.disabled)).toBe(true);
  });
});
