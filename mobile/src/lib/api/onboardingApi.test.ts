import assert from "node:assert/strict";
import test from "node:test";
import { URL, fileURLToPath } from "node:url";
import { hookHarness } from "../../features/assistant/assistantHookTestHarness";
import * as contracts from "../../features/onboarding/onboardingContracts";

test("onboarding uses the backend's GET/PUT/hide contract and rejects invalid responses", async () => {
  const requests: any[] = [];
  let response: any = {
    show_intro: true,
    suggestions_hidden: false,
    intro_version: 1,
    profile: null,
    topics: [],
    work_styles: [],
    recommendations: [],
  };
  const h = hookHarness<any>({
    "./http": {
      requestJson: async (path: string, options: any) => {
        requests.push([path, options]);
        return response;
      },
    },
    "../../features/onboarding/onboardingContracts": contracts,
  });
  const api = h.load(
    fileURLToPath(new URL("./onboardingApi.ts", import.meta.url)),
  ).onboardingApi as any;
  const controller = new AbortController();
  await api.get(controller.signal);
  assert.equal(requests[0][0], "/assistant/nyxagent/onboarding");
  assert.equal(requests[0][1].signal, controller.signal);
  const answers = {
    outcome: "completed",
    topics: ["inbox"],
    work_style: "solo",
  };
  await api.save(answers);
  assert.equal(requests[1][1].method, "PUT");
  assert.equal(requests[1][1].body, answers);
  await api.hideSuggestions();
  assert.equal(
    requests[2][0],
    "/assistant/nyxagent/onboarding/suggestions/hide",
  );
  assert.equal(requests[2][1].method, "POST");
  response = { invalid: true };
  await assert.rejects(api.get());
});
