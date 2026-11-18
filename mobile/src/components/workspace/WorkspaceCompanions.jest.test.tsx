import { useMemo } from "react";
import { fireEvent, render, screen } from "@testing-library/react-native";
import { StyleSheet, Text } from "react-native";
import { SidebarSectionHeader } from "./SidebarSectionHeader";
import { Section } from "./WorkspaceUI";
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

it("renders group headings with visual uppercase while preserving their localized strings", async () => {
  await render(<>
    <SettingsGroup title="Account"><Text>Settings rows</Text></SettingsGroup>
    <Section title="Usage analytics"><Text>Section rows</Text></Section>
    <HistorySectionHeader title="今天" />
  </>);
  for (const title of ["Account", "Usage analytics", "今天"]) {
    const heading = screen.getByRole("header", { name: title });
    expect(heading.props.children).toBe(title);
    expect(StyleSheet.flatten(heading.props.style)).toMatchObject({
      fontSize: 12,
      fontWeight: "500",
      textTransform: "uppercase",
      letterSpacing: 1.5,
    });
  }
});

it("reflows scaled section text and keeps the count and actions available as width or type changes", async () => {
  const toggle = jest.fn();
  const refresh = jest.fn();
  const create = jest.fn();
  await render(
    <SidebarSectionHeader
      label="Agents"
      section="agents"
      count="4 total"
      expanded
      onPress={toggle}
      createLabel="New agent"
      onCreate={create}
      createDisabled={false}
      onRefresh={refresh}
    />,
  );
  const viewport = screen.getByTestId("sidebar-section-viewport");
  const heading = screen.getByTestId("sidebar-section-heading");
  const label = screen.getByTestId("sidebar-section-label");
  const meta = screen.getByTestId("sidebar-section-meta");
  const toggleButton = screen.getByRole("button", { name: "Collapse agents" });
  const measure = async (id: string, width: number) => {
    const text = screen.getByTestId(id, { includeHiddenElements: true });
    await fireEvent(text, "textLayout", { nativeEvent: { lines: [{ width }] } });
  };
  // Native measurements include the current font, translated string, and
  // platform scaling curve. Exercise a phone drawer at 2x and larger widths.
  await fireEvent(viewport, "layout", { nativeEvent: { layout: { width: 276 } } });
  await measure("sidebar-section-label-measure", 132);
  await measure("sidebar-section-meta-measure", 90);
  expect(StyleSheet.flatten(heading.props.style).flexDirection).toBe("column");
  expect(StyleSheet.flatten(toggleButton.props.style).flexWrap).toBe("wrap");
  expect(StyleSheet.flatten(label.props.style).flexShrink).toBe(0);
  expect(label.props.numberOfLines).toBe(1);
  expect(meta).toHaveTextContent("4 total");
  expect(toggleButton).toHaveAccessibilityValue({ text: "4 total" });
  for (const name of ["Refresh agents", "New agent"]) {
    const button = screen.getByRole("button", { name });
    expect(StyleSheet.flatten(button.props.style)).toMatchObject({ width: 44, height: 44 });
    await fireEvent.press(button);
  }
  await fireEvent.press(toggleButton);
  expect(refresh).toHaveBeenCalledTimes(1);
  expect(create).toHaveBeenCalledTimes(1);
  expect(toggle).toHaveBeenCalledTimes(1);

  await fireEvent(viewport, "layout", { nativeEvent: { layout: { width: 500 } } });
  expect(StyleSheet.flatten(heading.props.style).flexDirection).toBe("row");
  // An accessibility text-size change on an already open drawer reflows it
  // again without deleting the count or either action.
  await measure("sidebar-section-label-measure", 296);
  expect(StyleSheet.flatten(heading.props.style).flexDirection).toBe("column");
  expect(screen.getByTestId("sidebar-section-meta")).toHaveTextContent("4 total");
  expect(screen.getByRole("button", { name: "Refresh agents" })).toBeEnabled();
  expect(screen.getByRole("button", { name: "New agent" })).toBeEnabled();
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
