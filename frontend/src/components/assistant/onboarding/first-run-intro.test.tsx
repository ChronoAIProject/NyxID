import { cleanup, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { useAuthStore } from "@/stores/auth-store";
import type { AssistantOnboarding } from "@/schemas/assistant-onboarding";
import { FirstRunIntro } from "./first-run-intro";
import { NextStepCards } from "./next-step-cards";

// The live replays render the real chat bubbles, which need the app's router.
vi.mock("./live-replay", () => ({
  LiveReplay: ({ label }: { label: string }) => <figure aria-label={label} />,
}));

const at = "2026-11-04T00:00:00Z";
let writes: { method: string; endpoint: string; body: unknown }[];
let client: QueryClient;

const step = (id: string, title: string) => ({
  id,
  title,
  description: `${title} description`,
  prompt: `Please ${title.toLowerCase()}`,
  icon: "mail",
});

const state = (overrides: Partial<AssistantOnboarding> = {}): AssistantOnboarding => ({
  show_intro: true,
  suggestions_hidden: false,
  intro_version: 1,
  profile: null,
  topics: [
    { id: "inbox", label: "Email & inbox", icon: "mail", work_styles: ["solo", "team", "business"] },
    { id: "research", label: "Research & news", icon: "search", work_styles: ["solo", "team"] },
    { id: "sales", label: "Sales & leads", icon: "target", work_styles: ["business"] },
  ],
  work_styles: [
    { id: "solo", label: "Just me", icon: "user" },
    { id: "team", label: "My team", icon: "users" },
    { id: "business", label: "My business", icon: "store" },
  ],
  recommendations: [step("connect_inbox", "Connect your inbox")],
  ...overrides,
});

const json = (value: unknown) =>
  new Response(JSON.stringify(value), { status: 200, headers: { "content-type": "application/json" } });

beforeEach(() => {
  useAuthStore.getState().setUser({
    id: "owner",
    email: "owner@example.com",
    display_name: "Owner",
    avatar_url: null,
    email_verified: true,
    mfa_enabled: false,
    is_admin: false,
    is_active: true,
    created_at: at,
    capabilities: { billing_available: false, enabled_features: [] },
  });
  client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  writes = [];
  globalThis.__nyxidAssistantHttpMock = ({ endpoint, init }) => {
    const method = init.method ?? "GET";
    const body = init.body ? (JSON.parse(String(init.body)) as unknown) : null;
    if (method !== "GET") writes.push({ method, endpoint, body });
    if (endpoint === "/assistant/nyxagent/onboarding" && method === "PUT") {
      return json(
        state({
          show_intro: false,
          recommendations: [step("research_specialist", "Create a Research specialist")],
        }),
      );
    }
    return json({});
  };
});

afterEach(() => {
  cleanup();
  globalThis.__nyxidAssistantHttpMock = undefined;
});

function renderIntro(
  onStart = vi.fn(),
  onOpenChange = vi.fn(),
  onTour?: () => void,
  entryPoint: "first_run" | "account_menu" = "account_menu",
) {
  render(
    <QueryClientProvider client={client}>
      <FirstRunIntro
        open
        state={state()}
        entryPoint={entryPoint}
        onOpenChange={onOpenChange}
        onStart={onStart}
        onTour={onTour}
      />
    </QueryClientProvider>,
  );
  return { onStart, onOpenChange };
}

it("a first-timer is greeted by name before the questions", async () => {
  const user = userEvent.setup();
  renderIntro(vi.fn(), vi.fn(), undefined, "first_run");
  expect(screen.getByRole("heading", { name: "Welcome, Owner" })).toBeTruthy();
  expect(screen.queryByLabelText(/Step 1 of/)).toBeNull();
  await user.click(screen.getByRole("button", { name: "Let's get started" }));
  expect(screen.getByRole("heading", { name: "Let's set NyxBot up for you" })).toBeTruthy();
  // The greeting is not a step to go back to.
  expect(screen.queryByRole("button", { name: "Back" })).toBeNull();
});

it("skipping from the greeting records a skipped intro", async () => {
  const user = userEvent.setup();
  const { onOpenChange } = renderIntro(vi.fn(), vi.fn(), undefined, "first_run");
  await user.click(screen.getByRole("button", { name: "Skip for now" }));
  expect(onOpenChange).toHaveBeenCalledWith(false);
  await waitFor(() =>
    expect(writes).toEqual([
      {
        method: "PUT",
        endpoint: "/assistant/nyxagent/onboarding",
        body: { outcome: "skipped", topics: [], work_style: null },
      },
    ]),
  );
});

it("skipping saves the picks as a skipped intro and closes", async () => {
  const user = userEvent.setup();
  const { onOpenChange } = renderIntro();
  await user.click(screen.getByRole("checkbox", { name: /Email & inbox/ }));
  await user.click(screen.getByRole("button", { name: "Skip intro" }));
  expect(onOpenChange).toHaveBeenCalledWith(false);
  await waitFor(() =>
    expect(writes).toEqual([
      {
        method: "PUT",
        endpoint: "/assistant/nyxagent/onboarding",
        body: { outcome: "skipped", topics: ["inbox"], work_style: null },
      },
    ]),
  );
});

it("finishing saves once and starts a ranked next step", async () => {
  const user = userEvent.setup();
  const { onStart, onOpenChange } = renderIntro();
  await user.click(screen.getByRole("checkbox", { name: /Research & news/ }));
  await user.click(screen.getByRole("radio", { name: /My team/ }));
  await user.click(screen.getByRole("button", { name: "Continue" }));
  expect(screen.getByRole("heading", { name: "Meet NyxBot, your chief of staff" })).toBeTruthy();
  await user.click(screen.getByRole("button", { name: "Next" }));
  await user.click(screen.getByRole("button", { name: "Next" }));
  expect(screen.getByRole("heading", { name: "You stay in control" })).toBeTruthy();
  await user.click(screen.getByRole("button", { name: "Next" }));
  const start = await screen.findByRole("button", { name: /Create a Research specialist/ });
  await user.click(start);
  expect(onOpenChange).toHaveBeenCalledWith(false);
  expect(onStart).toHaveBeenCalledWith(
    expect.objectContaining({ id: "research_specialist", prompt: "Please create a research specialist" }),
  );
  expect(writes).toEqual([
    {
      method: "PUT",
      endpoint: "/assistant/nyxagent/onboarding",
      body: { outcome: "completed", topics: ["research"], work_style: "team" },
    },
  ]);
});

it("the last step offers the on-screen tour", async () => {
  const user = userEvent.setup();
  const onTour = vi.fn();
  const { onOpenChange } = renderIntro(vi.fn(), vi.fn(), onTour);
  for (const name of ["Continue without picking", "Next", "Next", "Next"]) {
    await user.click(screen.getByRole("button", { name }));
  }
  await user.click(await screen.findByRole("button", { name: "Show me around" }));
  expect(onOpenChange).toHaveBeenCalledWith(false);
  expect(onTour).toHaveBeenCalledTimes(1);
  expect(writes.map((write) => (write.body as { outcome: string }).outcome)).toEqual(["completed"]);
});

it("topics follow who NyxBot is for and drop picks that no longer apply", async () => {
  const user = userEvent.setup();
  renderIntro();
  expect(screen.queryByRole("checkbox", { name: /Sales & leads/ })).toBeNull();
  await user.click(screen.getByRole("checkbox", { name: /Research & news/ }));
  await user.click(screen.getByRole("checkbox", { name: /Email & inbox/ }));
  await user.click(screen.getByRole("radio", { name: /My business/ }));
  // Leaving pills fade out first, then the list settles.
  await waitFor(() => expect(screen.queryByRole("checkbox", { name: /Research & news/ })).toBeNull());
  expect(screen.getByRole("checkbox", { name: /Email & inbox/ })).toBeTruthy();
  await user.click(screen.getByRole("checkbox", { name: /Sales & leads/ }));
  await user.click(screen.getByRole("button", { name: "Skip intro" }));
  await waitFor(() =>
    expect(writes[0]?.body).toEqual({ outcome: "skipped", topics: ["inbox", "sales"], work_style: "business" }),
  );
});

it("step bars jump anywhere; suggestions show at once and later changes save again", async () => {
  const user = userEvent.setup();
  renderIntro();
  await user.click(screen.getByRole("button", { name: /Step 5 of 5: First steps/ }));
  // Suggestions show while the save runs.
  expect(
    screen.getAllByRole("button", { name: /Connect your inbox|Create a Research specialist/ }).length,
  ).toBeGreaterThan(0);
  await user.click(screen.getByRole("button", { name: /Step 1 of 5: Your picks/ }));
  await user.click(screen.getByRole("checkbox", { name: /Email & inbox/ }));
  await user.click(screen.getByRole("button", { name: /Step 5 of 5: First steps/ }));
  await waitFor(() =>
    expect(writes.map((write) => (write.body as { topics: string[] }).topics)).toEqual([[], ["inbox"]]),
  );
});

it("arrow keys move between intro screens", async () => {
  const user = userEvent.setup();
  renderIntro();
  await user.click(screen.getByRole("checkbox", { name: /Email & inbox/ }));
  await user.keyboard("{ArrowRight}");
  expect(screen.getByRole("heading", { name: "Meet NyxBot, your chief of staff" })).toBeTruthy();
  await user.keyboard("{ArrowLeft}");
  expect(screen.getByRole("heading", { name: "Let's set NyxBot up for you" })).toBeTruthy();
  expect(screen.getByRole("checkbox", { name: /Email & inbox/ }).getAttribute("aria-checked")).toBe("true");
});

it("next-step cards start their step and carry no per-card dismiss", async () => {
  const user = userEvent.setup();
  const onStart = vi.fn();
  render(<NextStepCards steps={[step("connect_inbox", "Connect your inbox")]} onStart={onStart} />);
  expect(screen.queryByRole("button", { name: /Dismiss/ })).toBeNull();
  await user.click(screen.getByRole("button", { name: /Connect your inbox description/ }));
  expect(onStart).toHaveBeenCalledTimes(1);
});
