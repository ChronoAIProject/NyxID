import { createRoot, type Root } from "react-dom/client";
import { useState } from "react";
import {
  createMemoryHistory,
  createRootRoute,
  createRouter,
  RouterProvider,
} from "@tanstack/react-router";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { useApplyTheme } from "../../src/hooks/use-theme";
import { useThemeStore } from "../../src/stores/theme-store";
import { useAuthStore } from "../../src/stores/auth-store";
import { FirstRunIntro } from "../../src/components/assistant/onboarding/first-run-intro";
import { GuidedTour } from "../../src/components/assistant/onboarding/guided-tour";
import type { AssistantOnboarding } from "../../src/schemas/assistant-onboarding";
import "../../src/app.css";

const state: AssistantOnboarding = {
  show_intro: true,
  suggestions_hidden: false,
  intro_version: 1,
  profile: null,
  topics: [
    { id: "inbox", label: "Email & inbox", icon: "mail" },
    { id: "calendar", label: "Calendar & planning", icon: "calendar" },
    { id: "research", label: "Research & news", icon: "search" },
  ],
  work_styles: [
    { id: "solo", label: "Just me", icon: "user" },
    { id: "team", label: "My team", icon: "users" },
    { id: "business", label: "My business", icon: "store" },
  ],
  recommendations: [
    {
      id: "connect_inbox",
      title: "Connect your inbox",
      description: "Let NyxBot find the emails that need your attention.",
      prompt: "Please connect my inbox",
      icon: "mail",
    },
  ],
};
useAuthStore.getState().setUser({
  id: "display-owner",
  email: "owner@example.test",
  display_name: "Owner",
  avatar_url: null,
  email_verified: true,
  mfa_enabled: false,
  is_admin: false,
  is_active: true,
  created_at: "2026-11-04T00:00:00Z",
  capabilities: { billing_available: false, enabled_features: [] },
});
globalThis.__nyxidAssistantHttpMock = ({ endpoint }) =>
  endpoint === "/assistant/nyxagent/onboarding"
    ? new Response(JSON.stringify(state), {
        headers: { "content-type": "application/json" },
      })
    : undefined;

declare global {
  interface Window {
    onboardingFixture: {
      set: (scale: number, theme: "light" | "dark") => void;
    };
  }
}
window.onboardingFixture = {
  set: (textScale, mode) => useThemeStore.setState({ textScale, mode }),
};

export function Fixture() {
  useApplyTheme("assistant");
  const [open, setOpen] = useState(true);
  const [tour, setTour] = useState(false);
  const [started, setStarted] = useState("");
  return (
    <main className="p-4">
      <p data-started-step>{started}</p>
      <div data-tour="composer" className="mt-20 h-20">
        Message NyxBot
      </div>
      <aside data-tour="sidebar" className="h-40">
        Agents
      </aside>
      <FirstRunIntro
        open={open}
        state={state}
        entryPoint="first_run"
        onOpenChange={setOpen}
        onStart={(step) => setStarted(step.id)}
        onTour={() => setTour(true)}
      />
      {tour && <GuidedTour onClose={() => setTour(false)} />}
    </main>
  );
}
const client = new QueryClient({
  defaultOptions: { queries: { retry: false } },
});
const router = createRouter({
  routeTree: createRootRoute({ component: Fixture }),
  history: createMemoryHistory({ initialEntries: ["/"] }),
});
const container = document.getElementById("root")! as HTMLElement & { fixtureRoot?: Root };
const root = container.fixtureRoot ??= createRoot(container);
root.render(
  <QueryClientProvider client={client}>
    <RouterProvider router={router} />
  </QueryClientProvider>,
);
