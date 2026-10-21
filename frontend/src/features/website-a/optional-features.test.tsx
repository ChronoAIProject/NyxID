import {
  cleanup,
  fireEvent,
  render,
  screen,
  waitFor,
} from "@testing-library/react";
import { renderToString } from "react-dom/server";
import { afterEach, expect, it, vi } from "vitest";
import { NewsletterSignup } from "./newsletter-signup";
import { ContactChat } from "./contact-chat";
import { FooterStatus } from "./footer-status";
import { SolutionActivation } from "./solution-activation";
import { solutionSlugs, solutionStartTarget } from "./solution-availability";
import { resolveWebsiteRelease } from "./release-config";
let language: "en" | "zh" = "en",
  route = "/solutions/help-desk";
vi.mock("./website-context", () => ({
  useWebsite: () => ({ language, route, siteHref: (path: string) => path }),
}));
vi.mock("./site-link", () => ({
  SiteLink: ({
    href,
    children,
  }: {
    href: string;
    children: React.ReactNode;
  }) => <a href={href}>{children}</a>,
}));
const newsletter = {
  endpoint: "/api/public/newsletter",
  privacyUrl: "https://nyx.chrono-ai.fun/privacy",
  unsubscribeUrl: "https://status.example.test/unsubscribe",
};
const chat = { endpoint: "/api/public/chat" };
afterEach(() => {
  cleanup();
  vi.unstubAllGlobals();
  language = "en";
  route = "/solutions/help-desk";
});
it.each(Array.from({ length: 8 }, (_, i) => i))(
  "renders only the individually configured optional features (%i)",
  (mask) => {
    const fetcher = vi.fn();
    vi.stubGlobal("fetch", fetcher);
    const { container } = render(
      <>
        <NewsletterSignup config={mask & 1 ? newsletter : null} />
        <ContactChat config={mask & 2 ? chat : null} />
        <FooterStatus url={mask & 4 ? "https://status.example.test/" : null} />
      </>,
    );
    for (const [feature, bit] of [
      ["newsletter", 1],
      ["contact-chat", 2],
      ["footer-status", 4],
    ] as const)
      expect(
        Boolean(container.querySelector(`[data-website-feature="${feature}"]`)),
      ).toBe(Boolean(mask & bit));
    expect(fetcher).not.toHaveBeenCalled();
  },
);
it("keeps SSR forms inert before handlers attach", () => {
  const html = renderToString(
    <>
      <NewsletterSignup config={newsletter} />
      <ContactChat config={chat} />
    </>,
  );
  const root = document.createElement("div");
  root.innerHTML = html;
  for (const form of root.querySelectorAll("form")) {
    expect(form.querySelector("fieldset")).toHaveAttribute("disabled");
    expect(form.querySelector('button[type="submit"]')).toHaveAttribute(
      "disabled",
    );
  }
  expect(root.querySelector('[role="log"]')).toBeEmptyDOMElement();
});
it.each(["en", "zh"] as const)(
  "waits for newsletter acknowledgement and preserves truthful confirmation (%s)",
  async (locale) => {
    language = locale;
    let complete!: (response: Response) => void;
    const fetcher = vi.fn(
      (_url: string, _options: RequestInit) =>
        new Promise<Response>((resolve) => {
          complete = resolve;
        }),
    );
    vi.stubGlobal("fetch", fetcher);
    render(<NewsletterSignup config={newsletter} />);
    fireEvent.change(screen.getByRole("textbox"), {
      target: { value: "reader@example.test" },
    });
    fireEvent.submit(document.querySelector("form")!);
    await waitFor(() => expect(fetcher).toHaveBeenCalledTimes(1));
    expect(screen.queryByRole("status")).toBeNull();
    expect(fetcher.mock.calls[0]?.[0]).toBe(newsletter.endpoint);
    complete(
      new Response(JSON.stringify({ status: "pending_confirmation" }), {
        headers: { "Content-Type": "application/json" },
      }),
    );
    await screen.findByRole("status");
    expect(screen.getByRole("status")).toHaveTextContent(
      locale === "zh" ? "订阅尚未确认" : "not confirmed yet",
    );
  },
);
it("handles backend failure without a success claim or losing newsletter input", async () => {
  vi.stubGlobal(
    "fetch",
    vi.fn().mockResolvedValue(new Response("down", { status: 503 })),
  );
  render(<NewsletterSignup config={newsletter} />);
  fireEvent.change(screen.getByRole("textbox"), {
    target: { value: "reader@example.test" },
  });
  fireEvent.submit(document.querySelector("form")!);
  await screen.findByRole("alert");
  expect(screen.queryByRole("status")).toBeNull();
  expect(screen.getByRole("textbox")).toHaveValue("reader@example.test");
});
it("uses only a returned NyxBot reply, renders it as text and keeps errors recoverable", async () => {
  const fetcher = vi
    .fn()
    .mockResolvedValueOnce(
      new Response(JSON.stringify({ reply: "<b>Fixture reply</b>" })),
    )
    .mockResolvedValueOnce(new Response(JSON.stringify({ reply: "" })));
  vi.stubGlobal("fetch", fetcher);
  render(<ContactChat config={chat} />);
  fireEvent.change(screen.getByRole("textbox"), {
    target: { value: "Can I self-host?" },
  });
  fireEvent.submit(document.querySelector("form")!);
  await screen.findByText("<b>Fixture reply</b>");
  expect(document.querySelector('[role="log"] b')).toBeNull();
  const options = fetcher.mock.calls[0]![1] as RequestInit;
  expect(options.credentials).toBe("omit");
  expect(options.redirect).toBe("error");
  expect(JSON.parse(String(options.body))).toEqual({
    message: "Can I self-host?",
    locale: "en",
  });
  fireEvent.change(screen.getByRole("textbox"), {
    target: { value: "Keep this question" },
  });
  fireEvent.submit(document.querySelector("form")!);
  await screen.findByRole("alert");
  expect(screen.getByRole("textbox")).toHaveValue("Keep this question");
});
it("aborts public work on unmount", async () => {
  let signal: AbortSignal | undefined;
  vi.stubGlobal(
    "fetch",
    vi.fn((_url, options: RequestInit) => {
      signal = options.signal!;
      return new Promise<Response>(() => {});
    }),
  );
  const view = render(<ContactChat config={chat} />);
  fireEvent.change(screen.getByRole("textbox"), {
    target: { value: "A question" },
  });
  fireEvent.submit(document.querySelector("form")!);
  await waitFor(() => expect(signal).toBeDefined());
  view.unmount();
  expect(signal!.aborted).toBe(true);
});
it.each(solutionSlugs)(
  "opens only the accepted %s solution and shares the legacy gate",
  (slug) => {
    const release = resolveWebsiteRelease({
      VITE_WEBSITE_SOLUTION_ACTIVATIONS: JSON.stringify({
        [slug]: {
          target: `/assistant?setup=${slug}`,
          acceptance: "review/accepted",
        },
      }),
    });
    route = `/solutions/${slug}`;
    const view = render(<SolutionActivation targets={release.solutions} />);
    expect(screen.getByRole("link")).toHaveAttribute(
      "href",
      `/login?return_to=${encodeURIComponent(`/assistant?setup=${slug}`)}`,
    );
    view.unmount();
    for (const other of solutionSlugs) {
      expect(solutionStartTarget(`t=${other}`, release.solutions)).toBe(
        other === slug ? `/assistant?setup=${slug}` : `/solutions/${other}`,
      );
      if (other !== slug) {
        route = `/solutions/${other}`;
        const otherView = render(
          <SolutionActivation targets={release.solutions} />,
        );
        expect(screen.getByRole("button")).toBeDisabled();
        otherView.unmount();
      }
    }
  },
);
