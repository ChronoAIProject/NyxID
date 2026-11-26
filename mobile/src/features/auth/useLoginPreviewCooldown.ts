import { useEffect, useState } from "react";
import { previewCooldownMs } from "./loginRequestResolver";

export function useLoginPreviewCooldown() {
  const [remaining, setRemaining] = useState(previewCooldownMs);
  const sync = () => setRemaining(previewCooldownMs());
  useEffect(() => {
    if (!remaining) return;
    const timer = setTimeout(sync, remaining);
    return () => clearTimeout(timer);
  }, [remaining]);
  return { coolingDown: remaining > 0, sync };
}
