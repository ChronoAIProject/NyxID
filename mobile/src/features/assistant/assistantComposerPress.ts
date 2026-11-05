export const ASSISTANT_SEND_PRESS_GRACE_MS = 600;

/** A second Send tap must not become Stop when the composer changes in place. */
export function createAssistantComposerPressGuard(now = Date.now) {
  let protectedUntil = 0;
  return (action: "send" | "stop") => {
    const time = now();
    if (time < protectedUntil) return false;
    if (action === "send")
      protectedUntil = time + ASSISTANT_SEND_PRESS_GRACE_MS;
    return true;
  };
}
