import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { TooltipProvider } from "@/components/ui/tooltip";
import { actionCardGalleryFixtures } from "@/lib/assistant/gallery-fixtures";
import { AssistantActionCardsGalleryPage } from "./assistant-action-cards-gallery";

function renderGallery() {
  const queryClient = new QueryClient({
    defaultOptions: {
      queries: { retry: false },
      mutations: { retry: false },
    },
  });
  return render(
    <QueryClientProvider client={queryClient}>
      <TooltipProvider delayDuration={0}>
        <AssistantActionCardsGalleryPage />
      </TooltipProvider>
    </QueryClientProvider>,
  );
}

beforeEach(() => {
  window.history.replaceState({}, "", "/design/action-cards?mock");
});

afterEach(() => {
  window.history.replaceState({}, "", "/");
});

describe("AssistantActionCardsGalleryPage", () => {
  it("renders every seeded state and variant in mock mode", async () => {
    renderGallery();

    for (const specimen of actionCardGalleryFixtures.actionCards) {
      expect(screen.getByText(specimen.caption)).toBeInTheDocument();
    }
    for (const specimen of actionCardGalleryFixtures.approvalCards) {
      expect(screen.getByText(specimen.caption)).toBeInTheDocument();
    }
    expect(
      screen.getByText(actionCardGalleryFixtures.liveWizard.caption),
    ).toBeInTheDocument();

    expect(screen.getByText("api.internal.example.com")).toBeInTheDocument();
    expect(screen.getByText("Node n_77")).toBeInTheDocument();
    expect(screen.getByText("Org Current organisation")).toBeInTheDocument();
    expect(screen.getAllByText("k_1a2b")).toHaveLength(2);
    expect(screen.getAllByText("Service connected").length).toBeGreaterThan(0);
    await waitFor(() =>
      expect(
        screen.queryByText("Failed to load this conversation."),
      ).not.toBeInTheDocument(),
    );
  });

  it("walks the real mock OAuth dialog and resolves the live card", async () => {
    const user = userEvent.setup();
    renderGallery();
    const caption = screen.getByText(
      actionCardGalleryFixtures.liveWizard.caption,
    );
    const liveSpecimen = caption.parentElement;
    if (!liveSpecimen) throw new Error("live specimen wrapper missing");

    await user.click(
      within(liveSpecimen).getByRole("button", { name: "Connect GitHub" }),
    );
    expect(
      await screen.findByRole("heading", {
        name: "Configure routing for GitHub",
      }),
    ).toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: "Next: Connect" }));
    expect(
      await screen.findByRole("heading", { name: "Connect to GitHub" }),
    ).toBeInTheDocument();
    expect(screen.getByRole("button", { name: /Repositories/i })).toHaveAttribute(
      "aria-pressed",
      "true",
    );

    await user.click(
      screen.getByRole("button", { name: "Connect with GitHub" }),
    );
    expect(
      await screen.findByRole("heading", {
        name: /Service.*GitHub.*connected/i,
      }),
    ).toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: "Maybe later" }));
    await waitFor(() => {
      expect(
        within(liveSpecimen).getByText("Service connected"),
      ).toBeInTheDocument();
      expect(
        within(liveSpecimen).getByText(/service us_44/),
      ).toBeInTheDocument();
    });
  });

  it("renders a plain mock-mode hint when the query parameter is absent", () => {
    window.history.replaceState({}, "", "/design/action-cards");
    renderGallery();

    expect(
      screen.getByText(/This development gallery needs mock mode/i),
    ).toBeInTheDocument();
    expect(
      screen.getByRole("link", { name: "/design/action-cards?mock" }),
    ).toHaveAttribute("href", "/design/action-cards?mock");
  });
});

describe("action-card gallery fixtures", () => {
  it("contain only safe example content, never credential-shaped values", () => {
    const serialized = JSON.stringify(actionCardGalleryFixtures);
    const forbidden = [
      /lorem ipsum/i,
      /ghp_[A-Za-z0-9]{8,}/i,
      /github_pat_[A-Za-z0-9_]{8,}/i,
      /sk-[A-Za-z0-9_-]{8,}/i,
      /nyx(?:id)?_[a-z]+_[A-Za-z0-9_-]{8,}/i,
      /bearer\s+[A-Za-z0-9._~-]{8,}/i,
      /[0-9a-f]{8}-[0-9a-f]{4}-[1-5][0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}/i,
    ];

    for (const pattern of forbidden) {
      expect(serialized).not.toMatch(pattern);
    }
    expect(serialized).toContain("us_44");
    expect(serialized).toContain("n_77");
    expect(serialized).toContain("k_1a2b");
  });
});
