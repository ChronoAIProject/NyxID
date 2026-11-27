import { configure, render, screen } from "@testing-library/react-native";
import { VoiceRippleBanner } from "./VoiceRippleBanner";
import { defaultAvatar } from "../../../../shared/agent-avatars/model";
import { prepareVoiceRippleFrames } from "../../../../shared/agent-avatars/voice-ripple";
configure({ defaultIncludeHiddenElements: true });
let mockReduced = false,
  mockFocused = true;
jest.mock("../../lib/accessibility/useReducedMotion", () => ({
  useReducedMotion: () => mockReduced,
}));
jest.mock("./useAvatarAnimationActive", () => ({
  useAvatarAnimationActive: () => mockFocused,
}));
jest.mock("../../../../shared/agent-avatars/voice-ripple", () => ({
  ...jest.requireActual("../../../../shared/agent-avatars/voice-ripple"),
  prepareVoiceRippleFrames: jest.fn(async (palette: string) => ({
    palette,
    frames: Array.from({ length: 12 }, (_, i) => `data:image/png;base64,${i}`),
    still: "still",
    bytes: 100,
    buildMs: 10,
  })),
}));
const config = defaultAvatar("nyx", "nyxbot");
beforeEach(() => {
  mockReduced = false;
  mockFocused = true;
  jest.clearAllMocks();
});
test("prepares only an eligible banner and retains a complete poster while paused", async () => {
  const props = {
    config,
    status: "Listening",
    inputAccepting: true,
    outputPlaying: false,
  };
  const view = await render(<VoiceRippleBanner {...props} visible={false} />);
  expect(prepareVoiceRippleFrames).not.toHaveBeenCalled();
  expect(screen.getByTestId("voice-ripple-banner")).toBeTruthy();
  await view.rerender(<VoiceRippleBanner {...props} />);
  expect(prepareVoiceRippleFrames).toHaveBeenCalledTimes(1);
  expect(screen.getByTestId("voice-ripple-frame-0")).toBeTruthy();
  mockReduced = true;
  await view.rerender(<VoiceRippleBanner {...props} />);
  const calls = jest.mocked(prepareVoiceRippleFrames).mock.calls.length;
  await view.rerender(
    <VoiceRippleBanner
      {...props}
      config={{
        ...config,
        choices: { ...config.choices, ripple_palette: "sunrise" },
      }}
    />,
  );
  expect(prepareVoiceRippleFrames).toHaveBeenCalledTimes(calls);
  expect(screen.queryByTestId("voice-ripple-frame-0")).toBeNull();
});
test("reduced and unfocused voice fields do not read changing output levels", async () => {
  let reads = 0;
  const level = {
    get value() {
      reads++;
      return 0.9;
    },
  } as never;
  mockReduced = true;
  const props = {
    config,
    status: "Speaking",
    inputAccepting: false,
    outputPlaying: true,
    outputLevel: level,
  };
  const view = await render(<VoiceRippleBanner {...props} />);
  expect(reads).toBe(0);
  mockReduced = false;
  mockFocused = false;
  await view.rerender(<VoiceRippleBanner {...props} />);
  expect(reads).toBe(0);
  expect(prepareVoiceRippleFrames).not.toHaveBeenCalled();
});
