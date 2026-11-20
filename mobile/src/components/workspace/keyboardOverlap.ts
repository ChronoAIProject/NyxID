export type KeyboardFrame = { screenY: number; height: number };

export function visibleKeyboardFrame(
  frame: KeyboardFrame | undefined,
  windowHeight: number,
): KeyboardFrame | undefined {
  if (
    !frame ||
    !Number.isFinite(windowHeight) ||
    windowHeight <= 0 ||
    !Number.isFinite(frame.screenY) ||
    !Number.isFinite(frame.height) ||
    frame.height <= 0 ||
    frame.height > windowHeight ||
    frame.screenY < 0 ||
    frame.screenY >= windowHeight
  )
    return undefined;
  return {
    screenY: frame.screenY,
    height: Math.min(frame.height, windowHeight - frame.screenY),
  };
}

/** Reserve the intersection in matching coordinates. Android reports IME
 * height without its navigation bar, so its bottom also has to clear that gap.
 * iOS retains intersection-only handling for floating/undocked keyboards. */
export function keyboardOverlap(
  frame: { y: number; height: number },
  keyboard?: KeyboardFrame,
  platform = "ios",
): number {
  if (
    !keyboard ||
    !Number.isFinite(keyboard.screenY) ||
    !Number.isFinite(keyboard.height) ||
    keyboard.height <= 0 ||
    !Number.isFinite(frame.y) ||
    !Number.isFinite(frame.height) ||
    frame.height <= 0
  ) {
    return 0;
  }
  const bottom = frame.y + frame.height;
  if (bottom <= keyboard.screenY || frame.y >= keyboard.screenY + keyboard.height)
    return 0;
  return platform === "android"
    ? Math.min(frame.height, bottom - keyboard.screenY)
    : Math.min(bottom, keyboard.screenY + keyboard.height) -
        Math.max(frame.y, keyboard.screenY);
}
