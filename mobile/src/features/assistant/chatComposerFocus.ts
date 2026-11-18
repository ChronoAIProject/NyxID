/**
 * Tapping anywhere outside a focused chat composer blurs it (dismissing the
 * keyboard) without taking the tap: the workspace observes touches with
 * onTouchStart/onTouchEnd, which never claim the responder, so the tapped
 * link, button or rail control still acts on the first tap. The blur runs
 * when the touch ends, after the tapped control has received its release,
 * so the keyboard and layout move only once that press is settled.
 *
 * Pure module: no react-native import, so screens' tests can load it beside
 * their mocks.
 */
type Blurrable = { blur(): void };

let focusedComposer: Blurrable | null = null;
let touchInComposer = false;
let outsideTouch = false;

export const chatComposerFocus = {
  focused(input: Blurrable) {
    focusedComposer = input;
  },
  blurred(input: Blurrable) {
    if (focusedComposer === input) focusedComposer = null;
  },
  /** The composer's own onTouchStart: it fires before its ancestors'. */
  touchStartInComposer() {
    touchInComposer = true;
  },
  /** The workspace's onTouchStart, after any composer handler for this touch. */
  workspaceTouchStart() {
    outsideTouch = !touchInComposer;
    touchInComposer = false;
  },
  /** The workspace's onTouchEnd: blur only for a touch that began outside. */
  workspaceTouchEnd() {
    const blur = outsideTouch;
    outsideTouch = false;
    if (blur) focusedComposer?.blur();
  },
};
