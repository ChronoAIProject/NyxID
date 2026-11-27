import {
  act,
  configure,
  fireEvent,
  render,
  screen,
  waitFor,
} from "@testing-library/react-native";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import type { ReactElement, ReactNode } from "react";
import { AgentAvatarEditScreen } from "./AgentAvatarEditScreen";
import { AgentAvatarsFlag } from "../../lib/avatars/agentAvatarsFlag";
import { AVATAR_TEMPLATE, defaultAvatar } from "../../lib/avatars/agentAvatars";
import { agentSchema } from "./agentContracts";
import { translate } from "../../i18n/translate";

configure({ defaultIncludeHiddenElements: true });
const mockPngExport = jest.fn();
const mockShare = jest.fn(async (..._args: unknown[]) => {});
const mockFileWrite = jest.fn();
jest.mock("../../../../shared/agent-avatars/export", () => ({
  ...jest.requireActual("../../../../shared/agent-avatars/export"),
  pngAvatarAsync: (...args: unknown[]) => mockPngExport(...args),
}));
jest.mock("expo-file-system", () => ({
  Paths: { cache: "cache" },
  File: class {
    uri = "cache/face.png";
    write = mockFileWrite;
  },
}));
jest.mock("expo-sharing", () => ({
  shareAsync: (...args: unknown[]) => mockShare(...args),
}));

const mockAvatarProps = jest.fn();
const mockVoiceProps = jest.fn();
jest.mock("../../components/AgentAvatar", () => {
  const { View } = require("react-native"),
    React = require("react");
  return {
    AgentAvatar: (props: Record<string, unknown>) => {
      mockAvatarProps(props);
      return React.createElement(View, { accessibilityLabel: props.label });
    },
  };
});
jest.mock("../../components/avatar/VoiceRippleBanner", () => {
  const { View } = require("react-native"),
    React = require("react");
  return {
    VoiceRippleBanner: (props: Record<string, unknown>) => {
      mockVoiceProps(props);
      return React.createElement(View);
    },
  };
});
const ID = "6f1c2a8e-3b4d-4c5e-8f90-123456789abc";
const STORED = {
  template: AVATAR_TEMPLATE,
  choices: { head: "round", eyes: "visor", palette: "core-ocean" },
};
const FRESH = agentSchema.parse({
  id: ID,
  kind: "specialist",
  name: "researcher",
  description: "",
  status: "idle",
  can_maintain: true,
});
let mockAgent = FRESH;
let mockLatest = mockAgent;
const mockUpdates: unknown[] = [];
let mockHeaderRight: (() => ReactNode) | undefined;
const mockGoBack = jest.fn();

let mockLanguage: "en" | "zh-CN" = "en";
jest.mock("../../i18n/LanguageProvider", () => {
  const { translate } = jest.requireActual("../../i18n/translate");
  return {
    useAppLanguage: () => ({
      language: mockLanguage,
      t: (phrase: string, values?: Record<string, string | number>) =>
        translate(mockLanguage, phrase, values),
    }),
  };
});
jest.mock("../../theme/ThemeContext", () => {
  const { lightColors } = jest.requireActual("../../theme/mobileTheme");
  return { useTheme: () => ({ colors: lightColors, mode: "light" }) };
});
jest.mock("../../lib/accessibility/useReducedMotion", () => ({
  useReducedMotion: () => true,
  useReducedMotionPreference: () => true,
}));
jest.mock("@react-navigation/native", () => ({
  ...jest.requireActual("@react-navigation/native"),
  useIsFocused: () => true,
}));
jest.mock("../../hooks/useNetworkStatus", () => ({
  useNetworkStatus: () => ({ isConnected: true }),
}));
jest.mock("../../app/WorkspaceHeader", () => ({
  useWorkspaceBack: ({
    options,
  }: {
    options: { headerRight: () => ReactNode };
  }) => {
    mockHeaderRight = options.headerRight;
  },
}));
jest.mock("../../components/workspace/ScrollEdgeFades", () => {
  const { ScrollView } = jest.requireActual("react-native");
  return { PageScrollView: ScrollView };
});
jest.mock("../../lib/api/agentsApi", () => ({
  agentsApi: {
    detail: async () => ({
      agent: mockLatest ?? mockAgent,
      memory: [],
      threads: [],
    }),
    updateAvatar: async (id: string, body: unknown) => {
      mockUpdates.push({ id, body });
    },
  },
}));

async function open(agent = mockAgent) {
  mockAgent = agent;
  mockLatest = agent;
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false, gcTime: Infinity } },
  });
  const props = {
    navigation: { goBack: mockGoBack },
    route: { params: { agentId: ID } },
  } as unknown as Parameters<typeof AgentAvatarEditScreen>[0];
  await render(
    <QueryClientProvider client={client}>
      <AgentAvatarsFlag value>
        <AgentAvatarEditScreen {...props} />
      </AgentAvatarsFlag>
    </QueryClientProvider>,
  );
  await screen.findByLabelText(translate(mockLanguage, "Avatar preview"));
}

/** Presses the header's Save (the page hands it to the workspace header). */
async function save() {
  const button = mockHeaderRight!() as ReactElement<{
    enabled: boolean;
    onPress: () => void;
  }>;
  expect(button.props.enabled).toBe(true);
  await act(async () => button.props.onPress());
}

beforeEach(() => {
  mockLanguage = "en";
  mockAgent = FRESH;
  mockUpdates.length = 0;
  mockGoBack.mockClear();
  mockPngExport.mockReset();
  mockShare.mockClear();
  mockFileWrite.mockClear();
});

test("pickers are radio groups with the current choice checked", async () => {
  await open();
  const body = screen.getByLabelText("Head");
  expect(body.props.accessibilityRole).toBe("radiogroup");
  const deep = screen.getByLabelText("Round");
  expect(deep.props.accessibilityRole).toBe("radio");
  expect(deep.props.accessibilityState).toMatchObject({
    checked: true,
    selected: true,
  });
  expect(
    screen.getByLabelText("Preview expression").props.accessibilityRole,
  ).toBe("radiogroup");
});

test("a new choice saves through PATCH with the full avatar", async () => {
  await open();
  await fireEvent.press(screen.getByLabelText("Wink"));
  expect(screen.getByLabelText("Wink").props.accessibilityState).toMatchObject({
    checked: true,
  });
  await save();
  await waitFor(() => expect(mockUpdates).toHaveLength(1));
  expect(mockUpdates[0]).toEqual({
    id: ID,
    body: {
      avatar: {
        ...defaultAvatar(ID, "specialist"),
        choices: { ...defaultAvatar(ID, "specialist").choices, eyes: "wink" },
      },
    },
  });
  await waitFor(() => expect(mockGoBack).toHaveBeenCalled());
});

test("Reset to default clears a stored avatar", async () => {
  await open(agentSchema.parse({ ...mockAgent, avatar: STORED }));
  await fireEvent.press(screen.getByLabelText("Reset to default"));
  await save();
  await waitFor(() =>
    expect(mockUpdates).toEqual([{ id: ID, body: { avatar: null } }]),
  );
});

test("a change made elsewhere is refused, not overwritten", async () => {
  await open();
  mockLatest = agentSchema.parse({ ...mockAgent, avatar: STORED });
  await fireEvent.press(screen.getByLabelText("Wink"));
  await save();
  expect(
    await screen.findByText(
      "This avatar changed elsewhere. Go back and review it before saving.",
    ),
  ).toBeTruthy();
  expect(mockUpdates).toHaveLength(0);
});

test("viewers cannot change it", async () => {
  await open(agentSchema.parse({ ...mockAgent, can_maintain: false }));
  expect(
    screen.getByText("This agent's avatar can no longer be changed here."),
  ).toBeTruthy();
  expect(screen.getByLabelText("Wink").props.accessibilityState).toMatchObject({
    disabled: true,
  });
});

test("the editor is localized", async () => {
  mockLanguage = "zh-CN";
  await open();
  expect(screen.getByText("随机生成")).toBeTruthy();
  expect(screen.getByLabelText("预览表情")).toBeTruthy();
});

test("async export locks repeated taps, and unmount prevents a stale file/share", async () => {
  let finish!: (bytes: Uint8Array) => void;
  mockPngExport.mockImplementation(
    () =>
      new Promise<Uint8Array>((resolve) => {
        finish = resolve;
      }),
  );
  await open();
  await fireEvent.press(screen.getByText("Export saved avatar"));
  await fireEvent.press(screen.getByText("Export saved avatar"));
  expect(mockPngExport).toHaveBeenCalledTimes(1);
  expect(screen.getByText("Preparing…")).toBeTruthy();
  await screen.unmount();
  await act(async () => finish(new Uint8Array([1, 2, 3])));
  expect(mockFileWrite).not.toHaveBeenCalled();
  expect(mockShare).not.toHaveBeenCalled();
});
test("account changes cancel export before writing or opening the share sheet", async () => {
  let finish!: (bytes: Uint8Array) => void;
  mockPngExport.mockImplementation(
    () =>
      new Promise<Uint8Array>((resolve) => {
        finish = resolve;
      }),
  );
  await open();
  await fireEvent.press(screen.getByText("Export saved avatar"));
  const { advanceAuthEpoch } = require("../../lib/auth/authEpoch");
  advanceAuthEpoch();
  await act(async () => finish(new Uint8Array([1, 2, 3])));
  expect(mockFileWrite).not.toHaveBeenCalled();
  expect(mockShare).not.toHaveBeenCalled();
});

test("scrolling pauses both previews and returning preserves the explicit event identity", async () => {
  await open();
  await fireEvent(screen.getByTestId("avatar-editor-live-preview"), "layout", {
    nativeEvent: { layout: { x: 0, y: 0, width: 112, height: 112 } },
  });
  await fireEvent(screen.getByTestId("avatar-editor-voice-preview"), "layout", {
    nativeEvent: { layout: { x: 0, y: 130, width: 300, height: 100 } },
  });
  const key = mockAvatarProps.mock.lastCall?.[0].eventKey;
  const scroll = screen.getByTestId("avatar-editor-scroll");
  await fireEvent.scroll(scroll, {
    nativeEvent: {
      contentOffset: { y: 5000 },
      layoutMeasurement: { height: 300 },
    },
  });
  expect(mockAvatarProps.mock.lastCall?.[0].animationVisible).toBe(false);
  expect(mockVoiceProps.mock.lastCall?.[0].visible).toBe(false);
  await fireEvent.scroll(scroll, {
    nativeEvent: {
      contentOffset: { y: 0 },
      layoutMeasurement: { height: 300 },
    },
  });
  expect(mockAvatarProps.mock.lastCall?.[0].animationVisible).toBe(true);
  expect(mockVoiceProps.mock.lastCall?.[0].visible).toBe(true);
  expect(mockAvatarProps.mock.lastCall?.[0].eventKey).toBe(key);
});
