import assert from "node:assert/strict";
import test from "node:test";
import { featherIcon, onboardingSchema, topicsFor } from "./onboardingContracts";

test("parses the server onboarding state, keeping unknown answer values", () => {
  const state = onboardingSchema.parse({
    show_intro: false,
    intro_version: 1,
    profile: {
      outcome: "completed",
      topics: ["inbox", "gardening"],
      work_style: null,
      intro_version: 1,
      updated_at: "2026-11-04T00:00:00.000Z",
    },
    topics: [
      { id: "inbox", label: "Email & inbox", icon: "mail", work_styles: ["solo", "business"] },
      { id: "sales", label: "Sales & leads", icon: "target", work_styles: ["business"] },
    ],
    work_styles: [{ id: "solo", label: "Just me", icon: "user" }],
    recommendations: [
      {
        id: "connect_inbox",
        title: "Connect your inbox",
        description: "NyxBot triages mail.",
        prompt: "Help me connect my email.",
        icon: "mail",
      },
    ],
  });
  assert.deepEqual(state.profile?.topics, ["inbox", "gardening"]);
  assert.equal(state.recommendations[0]?.id, "connect_inbox");
  assert.deepEqual(topicsFor(state.topics, "solo").map((topic) => topic.id), ["inbox"]);
  assert.deepEqual(topicsFor(state.topics, "business").map((topic) => topic.id), ["inbox", "sales"]);
});

test("server icon names map to Feather, and unknown icons fall back", () => {
  assert.equal(featherIcon("headset"), "headphones");
  assert.equal(featherIcon("not-an-icon"), "star");
});
