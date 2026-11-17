const OWNER_KEY = "nyxid:resource-preferences-owner";

/** Legacy resource keys share one persisted owner; layout-only preferences are independent. */
export function bindAccountResourcePreferences(userId: string | null): void {
  try {
    if (userId && localStorage.getItem(OWNER_KEY) === userId) return;
    for (let index = localStorage.length - 1; index >= 0; index--) {
      const key = localStorage.key(index);
      if (key && (key === "nyxid:last-picked-agent-key" || key.startsWith("nyxid:desktop-collapsed:")))
        localStorage.removeItem(key);
    }
    if (userId) localStorage.setItem(OWNER_KEY, userId);
    else localStorage.removeItem(OWNER_KEY);
  } catch { /* Browser preferences can be unavailable. */ }
}
