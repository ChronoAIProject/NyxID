import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import type { ReactNode } from "react";
import { afterEach, expect, it, vi } from "vitest";
import { cleanup, render, screen } from "@testing-library/react";
import { TermsPage } from "./terms";
import { PrivacyPage } from "./privacy";

vi.mock("@tanstack/react-router", () => ({
  Link: ({ to, children }: { to: string; children: ReactNode }) => (
    <a href={to}>{children}</a>
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
