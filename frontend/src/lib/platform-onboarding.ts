import { z } from "zod";
import type { SetupTemplate } from "@/schemas/platform-onboarding";

export function pendingSetupRequest(
  actor: string,
  template: SetupTemplate,
): { id: string; clear: () => void } {
  const key = `nyxid:onboarding:create:${actor}:${template}`;
  const saved = sessionStorage.getItem(key);
  const id =
    saved && z.uuid().safeParse(saved).success ? saved : crypto.randomUUID();
  sessionStorage.setItem(key, id);
  return {
    id,
    clear: () => {
      if (sessionStorage.getItem(key) === id) sessionStorage.removeItem(key);
    },
  };
}

export function creditUnit(value: string | number): string {
  return Number(value) === 1 ? "credit" : "credits";
}
