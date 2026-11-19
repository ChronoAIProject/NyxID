import { useMemo } from "react";
import { fireEvent, render, screen } from "@testing-library/react-native";
import { StyleSheet, Text } from "react-native";
import { SidebarSectionHeader } from "./SidebarSectionHeader";
import { Section } from "./WorkspaceUI";
import { WorkspaceCornerSensorContext } from "./WorkspaceLayoutContext";
import { typeScale } from "../../theme/designTokens";
import { SettingsGroup } from "../settings/SettingsList";
import { HistorySectionHeader } from "../HistoryCard";

jest.mock("../ServiceIcon", () => ({ ServiceIcon: () => null }));
import {
  RightPanelContext,
  RightPanelProvider,
  RightPanelRegistration,
  RightPanelSlot,
} from "./WorkspaceRightPanel";

jest.mock("../../theme/ThemeContext", () => ({
  useTheme: () => ({
    colors: jest.requireActual("../../theme/mobileTheme").lightColors,
  }),
}));

it("P9 keeps base ordinary group headings and localized Chinese text", async () => {
  await render(<>
    <SettingsGroup title="Account"><Text>Settings rows</Text></SettingsGroup>
    <Section title="Usage analytics"><Text>Section rows</Text></Section>
    <HistorySectionHeader title="今天" />
  </>);
  for (const [title, expected] of [
    ["ACCOUNT", { ...typeScale.small, letterSpacing: 0.6 }],
    ["USAGE ANALYTICS", { fontSize: 12, letterSpacing: 0.6 }],
    ["今天", { ...typeScale.small, letterSpacing: 0.6 }],
  ] as const) {
    const heading = screen.getByRole("header", { name: title });
    expect(heading.props.children).toBe(title);
    expect(StyleSheet.flatten(heading.props.style)).toMatchObject({
      ...expected,
    });
  }
});

it("on the Duo keeps the intrinsic one-line label and 44pt controls when scaled metadata cannot fit", async () => {
  await render(
    <WorkspaceCornerSensorContext.Provider value={true}>
    <SidebarSectionHeader
      label="Agents"
      section="agents"
      count="1 total"
      expanded
      onPress={() => {}}
      createLabel="New agent"
      onCreate={() => {}}
      createDisabled={false}
      onRefresh={() => {}}
    />
    </WorkspaceCornerSensorContext.Provider>,
  );
  await fireEvent(screen.getByTestId("sidebar-section-viewport"), "layout", {
    nativeEvent: { layout: { width: 276 } },
  });
  await fireEvent(screen.getByTestId("sidebar-section-label-measure", { includeHiddenElements: true }), "textLayout", {
    nativeEvent: { lines: [{ width: 132 }] },
  });
  await fireEvent(
    screen.getByTestId("sidebar-section-meta-measure", {
      includeHiddenElements: true,
    }),
    "textLayout",
    { nativeEvent: { lines: [{ width: 90 }] } },
  );
  await fireEvent(screen.getByTestId("sidebar-section-chevron"), "layout", {
    nativeEvent: { layout: { width: 16 } },
  });
  for (const name of ["Refresh agents", "New agent"]) {
    const button = screen.getByRole("button", { name });
    await fireEvent(button, "layout", {
      nativeEvent: { layout: { width: 44 } },
    });
    expect(StyleSheet.flatten(button.props.style).width).toBe(44);
  }
  const label = screen.getByTestId("sidebar-section-label");
  expect(label.props.numberOfLines).toBe(1);
  expect(StyleSheet.flatten(label.props.style).flexShrink).toBe(1);
  expect(
    StyleSheet.flatten(
      screen.getByTestId("sidebar-section-meta", {
        includeHiddenElements: true,
      }).props.style,
    ).width,
  ).toBe(0);
  await fireEvent(screen.getByTestId("sidebar-section-viewport"), "layout", {
    nativeEvent: { layout: { width: 500 } },
  });
  expect(
    StyleSheet.flatten(
      screen.getByTestId("sidebar-section-meta", {
        includeHiddenElements: true,
      }).props.style,
    ).width,
  ).toBe(90);
  // A subsequent Dynamic Type change hides metadata again, preserving the label.
  await fireEvent(screen.getByTestId("sidebar-section-label-measure", { includeHiddenElements: true }), "textLayout", { nativeEvent: { lines: [{ width: 296 }] } });
  expect(
    StyleSheet.flatten(
      screen.getByTestId("sidebar-section-meta", {
        includeHiddenElements: true,
      }).props.style,
    ).width,
  ).toBe(0);
  expect(label.props.ellipsizeMode).toBe("tail");
  expect(label.props.accessibilityLabel).toBeUndefined();
});

it("registers an opt-in panel, replaces content and removes it when resized or the page leaves", async () => {
  const page = (enabled: boolean, text?: string) => (
    <RightPanelProvider enabled={enabled}>
      {text ? (
        <RightPanelRegistration>
          <Text>{text}</Text>
        </RightPanelRegistration>
      ) : null}
      <RightPanelSlot />
    </RightPanelProvider>
  );
  const rendered = await render(page(true, "Home groups"));
  expect(screen.getByTestId("workspace-right-panel")).toBeTruthy();
  expect(screen.getByText("Home groups")).toBeTruthy();
  await rendered.rerender(page(true, "Agent access and activity"));
  expect(screen.queryByText("Home groups")).toBeNull();
  expect(screen.getByText("Agent access and activity")).toBeTruthy();
  await rendered.rerender(page(false, "Agent access and activity"));
  expect(screen.queryByTestId("workspace-right-panel")).toBeNull();
  await rendered.rerender(page(true, "Agent access and activity"));
  expect(screen.getByTestId("workspace-right-panel")).toBeTruthy();
  await rendered.rerender(page(true));
  expect(screen.queryByTestId("workspace-right-panel")).toBeNull();
});

it("does not re-register a panel when the page renders with unchanged panel inputs", async () => {
  const setRightPanel = jest.fn();
  const context = { enabled: true, content: null, setRightPanel };
  function Page({ title, draft }: { title: string; draft: string }) {
    const panel = useMemo(() => <Text>{title}</Text>, [title]);
    return <RightPanelContext.Provider value={context}>
      <Text>{draft}</Text>
      <RightPanelRegistration>{panel}</RightPanelRegistration>
    </RightPanelContext.Provider>;
  }
  const page = await render(<Page title="Agent details" draft="" />);
  expect(setRightPanel).toHaveBeenCalledTimes(1);
  await page.rerender(<Page title="Agent details" draft="Typing a message" />);
  expect(setRightPanel).toHaveBeenCalledTimes(1);
  await page.rerender(<Page title="Updated details" draft="Typing a message" />);
  expect(setRightPanel).toHaveBeenCalledTimes(2);
  expect(setRightPanel).not.toHaveBeenCalledWith(null);
  await page.unmount();
  expect(setRightPanel).toHaveBeenLastCalledWith(null);
});
