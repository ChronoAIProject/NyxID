import type { PublicUseCase } from "@/schemas/public-help";
import { parseUseCaseIds } from "./search";

export function parseNyxbotStartSearch(search: Record<string, unknown>) {
  const usecases = parseUseCaseIds(search.usecases);
  return usecases ? { usecases } : {};
}

export function composeBusinessStarter(ids: string, useCases: readonly PublicUseCase[]): string {
  const selected = (parseUseCaseIds(ids)?.split(",") ?? []).flatMap((id) => {
    const item = useCases.find((item) => item.id === id);
    const prompt = item?.example_prompts[0];
    return item && prompt?.trim() ? [{ title: item.title, prompt }] : [];
  });
  if (selected.length === 1) return selected[0]?.prompt ?? "";
  if (!selected.length) return "";
  return "Set up these use cases for my business:\n" + selected
    .map((item) => `- ${item.title}: ${item.prompt}`).join("\n");
}
