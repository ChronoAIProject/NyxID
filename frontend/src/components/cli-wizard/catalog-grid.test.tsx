import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import type { PropsWithChildren } from "react";
import { beforeEach, describe, expect, it, vi } from "vitest";

// Issue #787: CatalogGrid is step 1 of the wizard's `service add` flow.
// These tests pin the user-facing contract the maintainer cares about:
//   - the catalog renders as selectable cards (Simple setup) + a single
//     Custom / self-hosted card (Advanced);
//   - typing in the search box fuzzy-filters the rendered cards;
//   - clicking a card propagates the picked slug to onSelect, and the
//     Custom card propagates the "__custom__" sentinel;
//   - the empty-search and load-error branches surface the right copy;
//   - oauth / device-code / ssh entries carry a flow badge.

const { mockGet } = vi.hoisted(() => ({
  mockGet: vi.fn(),
}));

vi.mock("@/lib/api-client", () => ({
  api: { get: mockGet },
  // CatalogGrid renders a distinct error string for ApiError vs unknown
  // errors, so the test double must be a real subclass to keep
  // `instanceof ApiError` true.
  ApiError: class ApiError extends Error {
    status: number;
    errorCode: number;
    constructor(
      status: number,
      response: { message: string; error_code: number },
    ) {
      super(response.message);
      this.name = "ApiError";
      this.status = status;
      this.errorCode = response.error_code;
    }
  },
}));

import { ApiError, api } from "@/lib/api-client";
import { CatalogGrid } from "./catalog-grid";

function createWrapper() {
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  });
  return function Wrapper({ children }: PropsWithChildren) {
    return (
      <QueryClientProvider client={queryClient}>{children}</QueryClientProvider>
    );
  };
}

const CATALOG = [
  {
    slug: "llm-openai",
    name: "OpenAI",
    description: "GPT models",
    provider_type: "api_key",
    requires_credential: true,
  },
  {
    slug: "github",
    name: "GitHub",
    description: "Repos and issues",
    provider_type: "oauth2",
  },
  {
    slug: "stripe-device",
    name: "Stripe",
    description: "Payments",
    provider_type: "device_code",
  },
  {
    slug: "my-ssh-box",
    name: "Production SSH",
    description: "Remote shell",
    service_type: "ssh",
  },
];

beforeEach(() => {
  mockGet.mockReset();
});

describe("CatalogGrid — rendering + selection", () => {
  it("hits /catalog?include_all=true and renders each entry as a card", async () => {
    mockGet.mockResolvedValue({ entries: CATALOG });

    render(<CatalogGrid onSelect={vi.fn()} />, { wrapper: createWrapper() });

    expect(await screen.findByText("OpenAI")).toBeInTheDocument();
    expect(api.get).toHaveBeenCalledWith("/catalog?include_all=true");
    // All four catalog names plus their descriptions render.
    expect(screen.getByText("GitHub")).toBeInTheDocument();
    expect(screen.getByText("Stripe")).toBeInTheDocument();
    expect(screen.getByText("Production SSH")).toBeInTheDocument();
    expect(screen.getByText("GPT models")).toBeInTheDocument();
    // The Advanced "Custom / self-hosted" card is always present.
    expect(screen.getByText(/Custom \/ self-hosted/i)).toBeInTheDocument();
  });

  it("falls back to res.services when res.entries is absent", async () => {
    // The query reads `res.entries ?? res.services ?? []`; pin the
    // services branch so a backend that returns the legacy shape still
    // renders cards.
    mockGet.mockResolvedValue({ services: [CATALOG[0]] });

    render(<CatalogGrid onSelect={vi.fn()} />, { wrapper: createWrapper() });

    expect(await screen.findByText("OpenAI")).toBeInTheDocument();
  });

  it("calls onSelect with the catalog slug when a card is clicked", async () => {
    const user = userEvent.setup();
    const onSelect = vi.fn();
    mockGet.mockResolvedValue({ entries: CATALOG });

    render(<CatalogGrid onSelect={onSelect} />, { wrapper: createWrapper() });

    await user.click(await screen.findByText("OpenAI"));

    expect(onSelect).toHaveBeenCalledWith("llm-openai");
  });

  it("calls onSelect with the __custom__ sentinel when the Custom card is clicked", async () => {
    const user = userEvent.setup();
    const onSelect = vi.fn();
    mockGet.mockResolvedValue({ entries: CATALOG });

    render(<CatalogGrid onSelect={onSelect} />, { wrapper: createWrapper() });

    // Wait for the catalog to settle so we're clicking the real card.
    await screen.findByText("OpenAI");
    await user.click(screen.getByText(/Custom \/ self-hosted/i));

    expect(onSelect).toHaveBeenCalledWith("__custom__");
  });

  it("badges oauth, device-code and ssh entries with their flow kind", async () => {
    mockGet.mockResolvedValue({ entries: CATALOG });

    render(<CatalogGrid onSelect={vi.fn()} />, { wrapper: createWrapper() });

    await screen.findByText("GitHub");
    // BADGE_LABEL only covers oauth / device-code / ssh; the api_key
    // entry (OpenAI) has none.
    expect(screen.getByText("OAuth")).toBeInTheDocument();
    expect(screen.getByText("Device code")).toBeInTheDocument();
    expect(screen.getByText("SSH")).toBeInTheDocument();
  });
});

describe("CatalogGrid — search filtering", () => {
  it("filters the rendered cards to entries matching the query", async () => {
    const user = userEvent.setup();
    mockGet.mockResolvedValue({ entries: CATALOG });

    render(<CatalogGrid onSelect={vi.fn()} />, { wrapper: createWrapper() });

    await screen.findByText("OpenAI");
    const list = screen.getByRole("list");
    expect(within(list).getAllByRole("listitem")).toHaveLength(4);

    await user.type(screen.getByLabelText("Search"), "github");

    // After filtering, only GitHub survives in the catalog list. (The
    // Custom card lives outside the role="list" container, so it is not
    // counted here.)
    await waitFor(() => {
      expect(within(list).getAllByRole("listitem")).toHaveLength(1);
    });
    expect(within(list).getByText("GitHub")).toBeInTheDocument();
    expect(within(list).queryByText("OpenAI")).not.toBeInTheDocument();
  });

  it("matches by display name as well as slug", async () => {
    const user = userEvent.setup();
    mockGet.mockResolvedValue({ entries: CATALOG });

    render(<CatalogGrid onSelect={vi.fn()} />, { wrapper: createWrapper() });

    await screen.findByText("OpenAI");
    // "Production" only appears in the name field of my-ssh-box, not its
    // slug — proves fuzzyScore runs against the name too.
    await user.type(screen.getByLabelText("Search"), "Production");

    const list = screen.getByRole("list");
    await waitFor(() => {
      expect(within(list).getByText("Production SSH")).toBeInTheDocument();
    });
    expect(within(list).queryByText("OpenAI")).not.toBeInTheDocument();
  });

  it("shows the no-match copy when the query matches nothing", async () => {
    const user = userEvent.setup();
    mockGet.mockResolvedValue({ entries: CATALOG });

    render(<CatalogGrid onSelect={vi.fn()} />, { wrapper: createWrapper() });

    await screen.findByText("OpenAI");
    await user.type(screen.getByLabelText("Search"), "zzzznotathing");

    expect(
      await screen.findByText("No services match your search."),
    ).toBeInTheDocument();
    expect(screen.queryByRole("list")).not.toBeInTheDocument();
  });
});

describe("CatalogGrid — flow meta-labels", () => {
  it("labels a no-auth entry (requires_credential:false) '1-click connect'", async () => {
    mockGet.mockResolvedValue({
      entries: [
        {
          slug: "internal-health",
          name: "Health Check",
          description: "No credentials needed",
          provider_type: "api_key",
          requires_credential: false,
        },
      ],
    });

    render(<CatalogGrid onSelect={vi.fn()} />, { wrapper: createWrapper() });

    await screen.findByText("Health Check");
    expect(screen.getByText("1-click connect")).toBeInTheDocument();
  });

  it("labels a requires_gateway_url entry 'URL + API key'", async () => {
    mockGet.mockResolvedValue({
      entries: [
        {
          slug: "openclaw",
          name: "OpenClaw",
          description: "Self-hosted gateway",
          provider_type: "api_key",
          requires_credential: true,
          requires_gateway_url: true,
        },
      ],
    });

    render(<CatalogGrid onSelect={vi.fn()} />, { wrapper: createWrapper() });

    await screen.findByText("OpenClaw");
    expect(screen.getByText("URL + API key")).toBeInTheDocument();
  });

  it("labels a token-exchange entry with the field count ('N fields')", async () => {
    mockGet.mockResolvedValue({
      entries: [
        {
          slug: "google-cloud",
          name: "Google Cloud",
          description: "Token exchange",
          provider_type: "api_key",
          requires_credential: true,
          token_exchange_credential_fields: ["client_id", "client_secret"],
        },
      ],
    });

    render(<CatalogGrid onSelect={vi.fn()} />, { wrapper: createWrapper() });

    await screen.findByText("Google Cloud");
    // shapeLabel interpolates the field-array length: 2 entries → "2 fields".
    expect(screen.getByText("2 fields")).toBeInTheDocument();
  });
});

describe("CatalogGrid — empty / loading / error states", () => {
  it("shows the empty-catalog copy when the API returns no entries", async () => {
    mockGet.mockResolvedValue({ entries: [] });

    render(<CatalogGrid onSelect={vi.fn()} />, { wrapper: createWrapper() });

    // Empty (no filter) takes the "Catalog is empty." branch, distinct
    // from the no-search-match branch above.
    expect(await screen.findByText("Catalog is empty.")).toBeInTheDocument();
  });

  it("shows a loading message while the catalog query is in flight", () => {
    // Never-resolving promise keeps isLoading true.
    mockGet.mockReturnValue(new Promise(() => {}));

    render(<CatalogGrid onSelect={vi.fn()} />, { wrapper: createWrapper() });

    expect(screen.getByText("Loading catalog…")).toBeInTheDocument();
  });

  it("surfaces the ApiError message and status on load failure", async () => {
    mockGet.mockRejectedValue(
      new ApiError(503, {
        error: "service_unavailable",
        message: "upstream down",
        error_code: 9001,
      }),
    );

    render(<CatalogGrid onSelect={vi.fn()} />, { wrapper: createWrapper() });

    expect(
      await screen.findByText(/Couldn't load the catalog: upstream down \(503\)/),
    ).toBeInTheDocument();
  });

  it("shows a generic error message for non-ApiError failures", async () => {
    mockGet.mockRejectedValue(new Error("boom"));

    render(<CatalogGrid onSelect={vi.fn()} />, { wrapper: createWrapper() });

    expect(
      await screen.findByText(
        "Couldn't load the catalog. Check the CLI logs for details.",
      ),
    ).toBeInTheDocument();
  });
});
