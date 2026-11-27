import { readFileSync } from "node:fs";
import { createRequire } from "node:module";
import { resolve } from "node:path";
import { transform } from "lightningcss";
import {
  act,
  cleanup,
  render,
  renderHook,
  screen,
} from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { useAuthStore } from "@/stores/auth-store";
import {
  AVATAR_HAPPY_MS,
  NYXBOT_AVATAR,
} from "@/lib/assistant/agent-avatar-model";
import {
  groupMemberMood,
  moodInputFor,
  settledReplyOf,
  useAgentMood,
} from "@/lib/assistant/use-agent-mood";
import { AgentAvatar, AgentAvatarStack } from "./nyxbot-agent-avatar";
import { avatarSchedulerInfo } from "./avatar/scheduler";
import { contrastRatio } from "@/lib/theme-colors";

function themeTokens(css: string, selector: string) {
  const start = css.indexOf(selector + " {");
  const end = css.indexOf("\n}", start);
  return [...css.slice(start, end).matchAll(/(--color-[\w-]+):\s*([^;]+);/g)]
    .map(([, key, value]) => [key!, value!.trim()] as const);
}

function badgeColor(token: string, tokens: Map<string, string>): string {
  const value = tokens.get(`--color-${token}`)!.replace(
    /var\((--color-[\w-]+)\)/g,
    (_, reference: string) => tokens.get(reference)!,
  );
  if (/^#[0-9a-f]{6}$/i.test(value)) return value;
  // Target a browser that needs an sRGB fallback for the authored color.
  const { code } = transform({
    filename: "badge.css",
    code: Buffer.from(`a { color: ${value}; }`),
    targets: { chrome: 88 << 16 },
    minify: true,
  });
  const hex = /color:(#[0-9a-f]{6})(?=[;}])/i.exec(code.toString())?.[1];
  if (!hex) throw new Error(`No sRGB badge color for ${token}`);
  return hex;
}

function signIn(features: string[]) {
  useAuthStore.setState({
    user: {
      id: "u1",
      email: "u1@example.test",
      display_name: "U",
      capabilities: { enabled_features: features },
    } as never,
  });
}

const RESEARCH = {
  id: "a1",
  name: "release-notes",
  kind: "specialist" as const,
  display_name: null,
};

beforeEach(() => signIn([]));
afterEach(() => {
  cleanup();
  useAuthStore.setState({ user: null });
  vi.useRealTimers();
});

describe("AgentAvatar with assistant:avatars off", () => {
  it("renders exactly today's mark and initials, ignoring stored avatars", () => {
    const view = render(
      <>
        <AgentAvatar agent={{ ...RESEARCH, avatar: NYXBOT_AVATAR }} mood="working" />
        <AgentAvatar
          agent={{ id: "n", name: "NyxBot", kind: "nyxbot" }}
          size="lg"
        />
      </>,
    );
    expect(screen.getByText("RN")).toBeTruthy();
    expect(view.container.querySelector("[data-agent-avatar]")).toBeNull();
    expect(view.container.querySelector("[data-avatar-activity]")).toBeNull();
    expect(view.container.querySelector("img")).not.toBeNull();
  });
});

describe("AgentAvatar with assistant:avatars on", () => {
  beforeEach(() => signIn(["assistant:avatars"]));

  it.each(["light", "dark"] as const)("uses badge colors at 4.5:1 or better in %s theme", (mode) => {
    const css = readFileSync(resolve(import.meta.dirname, "../../app.css"), "utf8");
    const tailwind = readFileSync(createRequire(import.meta.url).resolve("tailwindcss/theme.css"), "utf8");
    const tokens = new Map([
      ...themeTokens(tailwind, "@theme default"),
      ...themeTokens(css, "@theme"),
      ...themeTokens(css, `html.theme-${mode}`),
    ]);
    const states = ["thinking", "approval", "waiting", "failure"] as const;
    const { container } = render(
      <>{states.map((state) => <AgentAvatar key={state} agent={RESEARCH} mood={state} />)}</>,
    );
    for (const state of states) {
      const badge = container.querySelector(`[data-avatar-activity="${state}"]`)!;
      const classes = [...badge.classList];
      const token = classes.find((name) => name.startsWith("text-"))!.slice(5);
      const override = mode === "light"
        ? classes.find((name) => name.startsWith("light:text-"))?.slice("light:text-".length)
        : undefined;
      const disc = [...badge.querySelector("circle")!.classList]
        .find((name) => name.startsWith("fill-"))!.slice(5);
      const ink = badgeColor(override ?? token, tokens);
      const background = badgeColor(disc, tokens);
      expect(contrastRatio(ink, background), `${mode} ${state} ${ink} on ${background}`).toBeGreaterThanOrEqual(4.5);
    }
  });

  it("draws inline SVG stills at every size with no canvas or animation", () => {
    const view = render(
      <>
        {(["xs", "sm", "md", "lg", "xl"] as const).map((size) => (
          <AgentAvatar key={size} agent={RESEARCH} size={size} />
        ))}
      </>,
    );
    expect(
      view.container.querySelectorAll("svg[data-agent-avatar]"),
    ).toHaveLength(5);
    expect(view.container.querySelector("canvas")).toBeNull();
    expect(view.container.querySelector("[data-avatar-renderer]")).toBeNull();
    expect(screen.queryByText("RN")).toBeNull();
  });

  it("keeps the saved texture in compact SVG identities", () => {
    const agent = {
      ...RESEARCH,
      avatar: {
        template: "face-v1",
        choices: { ...NYXBOT_AVATAR.choices, surface: "ascii-braille" },
      },
    };
    const view = render(<AgentAvatar agent={agent} size="sm" />);
    expect(
      view.container.querySelector("svg")?.getAttribute("data-face-surface"),
    ).toBe("ascii-braille");
  });

  it("is decorative unless labelled, and never announces the mood", () => {
    const view = render(<AgentAvatar agent={RESEARCH} mood="retry" />);
    expect(
      view.container.querySelector("svg")?.getAttribute("aria-hidden"),
    ).toBe("true");
    cleanup();
    render(<AgentAvatar agent={RESEARCH} mood="retry" label="release-notes" />);
    expect(screen.getByRole("img", { name: "release-notes" })).toBeTruthy();
    expect(screen.queryByText(/stressed/i)).toBeNull();
  });

  it("animates only when asked, at 24px and up", () => {
    vi.stubGlobal("IntersectionObserver", undefined);
    vi.stubGlobal("requestAnimationFrame", () => 1);
    vi.stubGlobal("cancelAnimationFrame", () => undefined);
    const view = render(
      <>
        <AgentAvatar agent={RESEARCH} size="xl" animated mood="working" />
        <AgentAvatar agent={RESEARCH} size="sm" animated mood="working" />
      </>,
    );
    try {
      expect(view.container.querySelectorAll('[data-avatar-activity="working"]')).toHaveLength(2);
      expect(avatarSchedulerInfo().active).toBe(0);
      expect(view.container.querySelectorAll('[data-avatar-motion="natural"]')).toHaveLength(1);
      expect(view.container.querySelectorAll('[data-avatar-motion="static"]')).toHaveLength(1);
    } finally {
      view.unmount();
      vi.unstubAllGlobals();
    }
  });

  it("keeps the stack's cutout discs", () => {
    const view = render(
      <AgentAvatarStack
        agents={[
          RESEARCH,
          { ...RESEARCH, id: "a2" },
          { ...RESEARCH, id: "a3" },
          { ...RESEARCH, id: "a4" },
        ]}
        max={3}
      />,
    );
    expect(
      view.container.querySelectorAll("svg[data-agent-avatar]"),
    ).toHaveLength(3);
    expect(screen.getByText("+1")).toBeTruthy();
  });
});

describe("useAgentMood", () => {
  const running = {
    turn_id: "t",
    started_at: "",
    activities: [],
    attachments: [],
  } as never;
  const input = (activeTurn: unknown) =>
    moodInputFor({
      activeTurn: activeTurn as never,
      streaming: false,
      pendingOwnerAction: false,
    });
  type Reply = {
    id: string;
    outcome: "completed" | "failed" | "stopped";
  } | null;
  const render = () =>
    renderHook(
      ({ id, turn, reply }: { id: string; turn: unknown; reply: Reply }) =>
        useAgentMood(id, input(turn), reply),
      {
        initialProps: {
          id: "c1",
          turn: null as unknown,
          reply: { id: "r1", outcome: "completed" } as Reply,
        },
      },
    );

  it("celebrates a completion seen live, once", () => {
    vi.useFakeTimers();
    const { result, rerender } = render();
    expect(result.current).toBe("idle");
    rerender({
      id: "c1",
      turn: running,
      reply: { id: "r1", outcome: "completed" },
    });
    expect(result.current).toBe("thinking");
    // The turn clears before its reply arrives: stay neutral.
    rerender({
      id: "c1",
      turn: null,
      reply: { id: "r1", outcome: "completed" },
    });
    expect(result.current).toBe("idle");
    rerender({
      id: "c1",
      turn: null,
      reply: { id: "r2", outcome: "completed" },
    });
    expect(result.current).toBe("success");
    act(() => vi.advanceTimersByTime(AVATAR_HAPPY_MS + 10));
    expect(result.current).toBe("idle");
  });

  it("settles a stopped turn to idle", () => {
    const { result, rerender } = render();
    rerender({
      id: "c1",
      turn: running,
      reply: { id: "r1", outcome: "completed" },
    });
    rerender({ id: "c1", turn: null, reply: { id: "r2", outcome: "stopped" } });
    expect(result.current).toBe("idle");
  });

  it("never flashes happy before a failure that arrives late", () => {
    const { result, rerender } = render();
    rerender({
      id: "c1",
      turn: running,
      reply: { id: "r1", outcome: "completed" },
    });
    rerender({
      id: "c1",
      turn: null,
      reply: { id: "r1", outcome: "completed" },
    });
    expect(result.current).toBe("idle");
    rerender({ id: "c1", turn: null, reply: { id: "r2", outcome: "failed" } });
    expect(result.current).toBe("failure");
  });

  it("a stopped or failed reply re-saved under a new id never celebrates", () => {
    // NyxAgent saves a completed reply under its streamed id; only a forced
    // stop or reclaim can save under a new one, and those are not completions.
    const { result, rerender } = render();
    rerender({
      id: "c1",
      turn: running,
      reply: { id: "r1", outcome: "completed" },
    });
    rerender({
      id: "c1",
      turn: null,
      reply: { id: "streamed", outcome: "failed" },
    });
    expect(result.current).toBe("failure");
    rerender({
      id: "c1",
      turn: null,
      reply: { id: "saved", outcome: "failed" },
    });
    expect(result.current).toBe("failure");
    rerender({
      id: "c1",
      turn: running,
      reply: { id: "saved", outcome: "failed" },
    });
    rerender({
      id: "c1",
      turn: null,
      reply: { id: "stop-a", outcome: "stopped" },
    });
    rerender({
      id: "c1",
      turn: null,
      reply: { id: "stop-b", outcome: "stopped" },
    });
    expect(result.current).toBe("idle");
  });

  it("does not celebrate a thread opened after it finished, or a thread switch", () => {
    const { result, rerender } = render();
    rerender({ id: "c2", turn: running, reply: null });
    rerender({
      id: "c3",
      turn: null,
      reply: { id: "x", outcome: "completed" },
    });
    expect(result.current).toBe("idle");
  });

  it("derives a working group member's mood from the group's pending cards", () => {
    expect(groupMemberMood("m1", [])).toBe("thinking");
    expect(groupMemberMood("m1", [{ agent_id: "m2" }])).toBe("thinking");
    expect(groupMemberMood("m1", [{ agent_id: "m1" }])).toBe("approval");
  });

  it("maps the live turn and transcript the page already holds", () => {
    expect(
      moodInputFor({
        activeTurn: {
          ...(running as object),
          activities: [{ status: "running" }],
          reply: { text: "Hi" },
        } as never,
        streaming: true,
        pendingOwnerAction: false,
      }).activeTurn,
    ).toMatchObject({ activityStatuses: ["running"], replyStreaming: true });
    // The page's own stream counts before the server row shows the turn.
    expect(
      moodInputFor({
        activeTurn: null,
        streaming: true,
        pendingOwnerAction: false,
      }).activeTurn,
    ).toEqual(expect.objectContaining({ replyStreaming: false }));
    const message = (id: string, role: string, status: string) =>
      ({ id, role, status, content: "", timestamp: 0 }) as never;
    expect(
      settledReplyOf([
        message("a", "assistant", "complete"),
        message("u", "user", "complete"),
      ]),
    ).toEqual({ id: "a", outcome: "completed" });
    expect(
      settledReplyOf([
        message("a", "assistant", "stopped"),
        message("b", "assistant", "streaming"),
      ]),
    ).toEqual({ id: "a", outcome: "stopped" });
    expect(settledReplyOf([message("a", "assistant", "error")])).toEqual({
      id: "a",
      outcome: "failed",
    });
    expect(settledReplyOf([])).toBeNull();
  });
});
