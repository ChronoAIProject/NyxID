import { requestJson } from "./http";
import {
  onboardingSchema,
  suggestionSetupSchema,
  type OnboardingAnswers,
} from "../../features/onboarding/onboardingContracts";

const ROOT = "/assistant/nyxagent/onboarding";

export const onboardingApi = {
  async suggestionSetup(id: string) {
    return suggestionSetupSchema.parse(await requestJson(`${ROOT}/suggestions/${encodeURIComponent(id)}`));
  },
  async get(signal?: AbortSignal) {
    return onboardingSchema.parse(await requestJson(ROOT, { signal }));
  },
  /** Finish or skip the intro; replaces earlier answers. */
  async save(answers: OnboardingAnswers) {
    return onboardingSchema.parse(
      await requestJson(ROOT, { method: "PUT", body: answers }),
    );
  },
  /** Hides the home's suggestions until the intro is retaken. */
  async hideSuggestions() {
    return onboardingSchema.parse(
      await requestJson(`${ROOT}/suggestions/hide`, { method: "POST" }),
    );
  },
};
