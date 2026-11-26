import React, {
  Component,
  useCallback,
  useRef,
  useState,
  type ReactNode,
} from "react";
import { Text } from "react-native";
import { render, screen } from "@testing-library/react-native";
import {
  useWorkspaceBack,
  WorkspaceNavigationGuardContext,
  type WorkspaceNavigationGuardChange,
} from "./WorkspaceHeader";

let mockFocused = true;
const mockNavigation = {
  setOptions: jest.fn(),
  goBack: jest.fn(),
  dispatch: jest.fn(),
};
jest.mock("@react-navigation/native", () => ({
  useIsFocused: () => mockFocused,
  useNavigation: () => mockNavigation,
  usePreventRemove: jest.fn(),
}));
jest.mock("../theme/ThemeContext", () => ({
  useTheme: () => ({ colors: {} }),
}));
jest.mock("../i18n/LanguageProvider", () => ({
  useAppLanguage: () => ({ t: (phrase: string) => phrase }),
}));
jest.mock("../lib/accessibility/workspaceFocus", () => ({
  workspaceFocus: {},
}));
jest.mock("../lib/accessibility/WorkspaceInteractionSurface", () => ({
  WorkspaceInteractionSurface: "View",
}));

class Boundary extends Component<{ children: ReactNode }, { failed: boolean }> {
  override state = { failed: false };
  static getDerivedStateFromError() {
    return { failed: true };
  }
  override render() {
    return this.state.failed ? <Text>Recovered</Text> : this.props.children;
  }
}
function Probe({
  disabled,
  crash = false,
}: {
  disabled: boolean;
  crash?: boolean;
}) {
  useWorkspaceBack({ active: false, disabled, onBack: mockNavigation.goBack });
  if (crash) throw new Error("test child render failure");
  return <Text>Page</Text>;
}
function Registry({ children }: { children: ReactNode }) {
  const guards = useRef(new Set<symbol>());
  const [count, setCount] = useState(0);
  const changed = useCallback<WorkspaceNavigationGuardChange>(
    (id, blocked, critical) => {
      if (blocked && critical) guards.current.add(id);
      else guards.current.delete(id);
      setCount(guards.current.size);
    },
    [],
  );
  return (
    <WorkspaceNavigationGuardContext.Provider value={changed}>
      <Text testID="critical-count">{count}</Text>
      {children}
    </WorkspaceNavigationGuardContext.Provider>
  );
}

beforeEach(() => {
  mockFocused = true;
});

for (const release of ["unmount", "focus loss", "write settles"] as const) {
  it(`releases a critical guard and re-renders its parent on ${release}`, async () => {
    const page = (visible = true, disabled = true) => (
      <Registry>{visible ? <Probe disabled={disabled} /> : null}</Registry>
    );
    const view = await render(page());
    expect(screen.getByTestId("critical-count").props.children).toBe(1);
    if (release === "focus loss") mockFocused = false;
    await view.rerender(
      page(release !== "unmount", release !== "write settles"),
    );
    expect(screen.getByTestId("critical-count").props.children).toBe(0);
    await view.unmount();
  });
}

it("an error boundary unmount releases the critical guard and re-renders the surviving parent", async () => {
  const error = jest.spyOn(console, "error").mockImplementation(() => {});
  try {
    const page = (crash: boolean) => (
      <Registry>
        <Boundary>
          <Probe disabled crash={crash} />
        </Boundary>
      </Registry>
    );
    const view = await render(page(false));
    expect(screen.getByTestId("critical-count").props.children).toBe(1);
    await view.rerender(page(true));
    expect(screen.getByText("Recovered")).toBeTruthy();
    expect(screen.getByTestId("critical-count").props.children).toBe(0);
    await view.unmount();
  } finally {
    error.mockRestore();
  }
});
