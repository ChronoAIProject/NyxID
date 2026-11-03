import { requestJson } from "./http";
import {
  onboardingSchema,
  type OnboardingAnswers,
} from "../../features/onboarding/onboardingContracts";

const ROOT = "/assistant/nyxagent/onboarding";

export const onboardingApi = {
  async get(signal?: AbortSignal) {
    return onboardingSchema.parse(await requestJson(ROOT, { signal }));
  },
  /** Finish or skip the intro; replaces earlier answers. */
  async save(answers: OnboardingAnswers) {
    return onboardingSchema.parse(
      await requestJson(ROOT, { method: "PUT", body: answers }),
    );
  },
  /** Hides one next step for good; returns the refreshed state. */
  async dismiss(id: string) {
    return onboardingSchema.parse(
      await requestJson(
        `${ROOT}/recommendations/${encodeURIComponent(id)}/dismiss`,
        { method: "POST" },
      ),
    );
  },
};
