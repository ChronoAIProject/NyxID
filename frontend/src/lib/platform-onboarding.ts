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

const NON_CHAT_MODEL =
  /image|embed|tts|whisper|moderation|dall-e|audio|realtime|transcri/i;
const TOOLING_MODEL = /codex|review/i;

/** Chat models a public guide can answer with; image/audio/embedding models are hidden. */
export function guideChatModels(models: string[]): string[] {
  return models.filter((model) => !NON_CHAT_MODEL.test(model));
}

/** A cheap general chat model: a "mini" first, then any non-tooling model. */
export function defaultGuideModel(models: string[]): string {
  const chat = guideChatModels(models).filter((m) => !TOOLING_MODEL.test(m));
  return chat.find((model) => /mini/i.test(model)) ?? chat[0] ?? "";
}

export function defaultGuideService(
  services: { id: string; name: string }[],
): string {
  return (
    (
      services.find((service) => /chrono llm/i.test(service.name)) ??
      services[0]
    )?.id ?? ""
  );
}
