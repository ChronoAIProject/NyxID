import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import type { ReactNode } from "react";
import { afterEach, expect, it, vi } from "vitest";
import { cleanup, render, screen } from "@testing-library/react";
import { TermsPage } from "./terms";
import { PrivacyPage } from "./privacy";
import { contrastRatio, DEFAULT_COLORS } from "@/lib/theme-colors";

vi.mock("@tanstack/react-router", () => ({
  Link: ({ to, children, className }: { to: string; children: ReactNode; className?: string }) => (
    <a href={to} className={className}>{children}</a>
  ),
}));
vi.mock("@/hooks/use-logo-href", () => ({ useLogoHref: () => "/" }));
vi.mock("@/hooks/use-theme", () => ({
  useApplyTheme: () => {},
  useResolvedTheme: () => "dark",
}));

const documents = {
  "/legal/terms.md": readFileSync(resolve("public/legal/terms.md"), "utf8"),
  "/legal/privacy.md": readFileSync(resolve("public/legal/privacy.md"), "utf8"),
};

function servePublishedDocuments() {
  const fetch = vi.fn(async (url: string) => {
    const body = documents[url as keyof typeof documents];
    return new Response(body ?? "Not found", { status: body ? 200 : 404 });
  });
  vi.stubGlobal("fetch", fetch);
  return fetch;
}

afterEach(() => {
  cleanup();
  vi.unstubAllGlobals();
  document.documentElement.className = "";
});

it("renders the published hosted-AI and paid-service terms with a public privacy link", async () => {
  const fetch = servePublishedDocuments();
  render(<TermsPage />);

  await screen.findByRole("heading", {
    name: "4.1 Hosted AI Features and Provider Connections",
  });
  expect(fetch).toHaveBeenCalledWith("/legal/terms.md", { cache: "no-store" });
  expect(screen.getByRole("heading", { name: "14.1 Applicable Fees" })).toBeVisible();
  expect(screen.getByText(/NyxID supports free and paid features/)).toBeVisible();
  expect(screen.getByText(/Full access bypasses confirmation requirements/)).toBeVisible();
  for (const link of screen.getAllByRole("link", { name: "Privacy Policy" })) {
    expect(link).toHaveAttribute("href", "/privacy");
  }
  expect(document.body).not.toHaveTextContent("No Inherent AI in the App");
  expect(document.body).not.toHaveTextContent("ChronoAI does not charge fees");
});

it("renders hosted content, encryption limits, recipients and incomplete-erasure disclosures", async () => {
  const fetch = servePublishedDocuments();
  render(<PrivacyPage />);

  await screen.findByRole("heading", { name: "6. Data Retention" });
  expect(fetch).toHaveBeenCalledWith("/legal/privacy.md", { cache: "no-store" });
  expect(screen.getByText(/Assistant file payloads, extracted upload text/)).toHaveTextContent(
    "conversation text, titles, memory notes, working-context summaries and feedback are stored as application data",
  );
  expect(screen.getByText(/This inventory describes supported data flows/)).toHaveTextContent(
    "not a claim that every listed service is enabled or acts as our sub-processor",
  );
  expect(screen.getByText(/Reply feedback, including optional comments/)).toHaveTextContent(
    "users with permission to read administrative usage reports, including Usage Viewers",
  );
  expect(screen.getByText(/NyxID requests revocation/)).toHaveTextContent(
    "Deletion does not yet cover every stored record",
  );
  expect(screen.getByText(/Software defaults are 24 hours/)).toHaveTextContent(
    "not a promise of the hosted deployment's current settings",
  );
  expect(screen.getByText(/Optional product analytics\./).closest("p")).toHaveTextContent(
    "defaults off",
  );
  expect(screen.getByRole("link", { name: "Terms of Use" })).toHaveAttribute("href", "/terms");
  expect(screen.getAllByRole("link", { name: "contact@chrono-ai.fun" })[0]).toHaveAttribute(
    "href",
    "mailto:contact@chrono-ai.fun",
  );
});

it.each([
  { Page: TermsPage, path: "/legal/terms.md", title: "Terms of Service" },
  { Page: PrivacyPage, path: "/legal/privacy.md", title: "Privacy Policy" },
])("keeps the canonical $title document reachable if page loading fails", async ({ Page, path }) => {
  vi.stubGlobal("fetch", vi.fn().mockResolvedValue(new Response("Unavailable", { status: 503 })));
  render(<Page />);
  expect(await screen.findByRole("link", { name: "View source" })).toHaveAttribute("href", path);
  expect(screen.getByRole("link", { name: "Back to NyxID" })).toHaveAttribute("href", "/");
});

it.each([
  { Page: TermsPage, theme: "dark" as const },
  { Page: TermsPage, theme: "light" as const },
  { Page: PrivacyPage, theme: "dark" as const },
  { Page: PrivacyPage, theme: "light" as const },
])("uses readable semantic prose and underlined links in $theme", async ({ Page, theme }) => {
  servePublishedDocuments();
  document.documentElement.className = `theme-${theme}`;
  const { container } = render(<Page />);
  await screen.findByRole("heading", { name: Page === PrivacyPage ? "6. Data Retention" : "14.1 Applicable Fees" });
  const card = container.querySelector(".legal-prose")!;
  const colors = DEFAULT_COLORS[theme];
  const textColor = (element: Element): string => {
    for (let node: Element | null = element; node; node = node.parentElement) {
      if (node.classList.contains("text-foreground")) return colors.foreground;
      if (node.classList.contains("text-muted-foreground")) return colors["muted-foreground"];
    }
    throw new Error(`Missing semantic color for ${element.tagName}`);
  };
  expect(card).toHaveClass("text-foreground", "bg-card");
  for (const element of card.querySelectorAll("p,li,h1,h2,h3,h4,h5,h6,strong,em,del,code,pre,blockquote,th,td,a")) {
    expect(contrastRatio(textColor(element), colors.card), element.tagName).toBeGreaterThanOrEqual(4.5);
  }
  const links = [...card.querySelectorAll("a"), screen.getByRole("link", { name: "Back to NyxID" })];
  expect(links.length).toBeGreaterThan(1);
  for (const link of links) {
    expect(link).toHaveClass("text-foreground", "underline", "hover:text-muted-foreground");
    for (const background of [colors.card, colors.background]) {
      expect(contrastRatio(colors.foreground, background)).toBeGreaterThanOrEqual(4.5);
      expect(contrastRatio(colors["muted-foreground"], background)).toBeGreaterThanOrEqual(4.5);
    }
  }
});
