import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import type { ActionCardContentBlock } from "@/types/assistant";
import { ActionCard } from "./action-card";

vi.mock("@/components/service-icon", () => ({
  ServiceIcon: ({ slug }: { readonly slug: string }) => <span>{slug}</span>,
}));

vi.mock("@/components/dashboard/add-key-dialog", () => ({
  AddKeyDialog: ({
    open,
    onOpenChange,
    prefillSlug,
    prefillCustom,
    onSuccess,
  }: {
    readonly open: boolean;
    readonly onOpenChange: (open: boolean) => void;
    readonly prefillSlug?: string;
    readonly prefillCustom?: { readonly name?: string };
    readonly onSuccess?: (result: { readonly userServiceId: string }) => void;
  }) =>
    open ? (
      <div
        role="dialog"
        data-prefill={prefillSlug ?? prefillCustom?.name ?? ""}
      >
        <button
          type="button"
          onClick={() =>
            onSuccess?.({
              userServiceId: "00000000-0000-4000-8000-000000000123",
            })
          }
        >
          Finish mock connection
        </button>
        <button type="button" onClick={() => onOpenChange(false)}>
          Dismiss mock connection
        </button>
      </div>
    ) : null,
}));

function catalogBlock(
  overrides: Partial<ActionCardContentBlock> = {},
): ActionCardContentBlock {
  return {
    type: "action_card",
    block_id: "action-card-1",
    action: "service.connect",
    action_request_id: "act-1",
    origin_turn_id: "turn-origin-1",
    params: {
      variant: "catalog",
      service_slug: "api-github",
      requested_scopes: ["repo"],
      via_node_id: "node-1",
      target_org_id: "org-1",
    },
    status: "pending",
    outcome_note: "",
    ...overrides,
  };
}

describe("ActionCard", () => {
  it("renders owned consent copy and opens the prefilled connect journey", async () => {
    const user = userEvent.setup();
    const onProgress = vi.fn();
    const onResolve = vi.fn();
    render(
      <ActionCard
        block={catalogBlock()}
        onProgress={onProgress}
        onResolve={onResolve}
      />,
    );

    expect(
      screen.getByRole("heading", { name: "Connect GitHub" }),
    ).toBeInTheDocument();
    expect(screen.getByText("repo")).toBeInTheDocument();
    expect(screen.getByText("Node node-1")).toBeInTheDocument();
    expect(screen.getByText("Org org-1")).toBeInTheDocument();
    expect(screen.getByText(/credential stays in NyxID/i)).toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: "Connect GitHub" }));
    expect(onProgress).toHaveBeenCalledWith("action-card-1", true);
    expect(screen.getByRole("dialog")).toHaveAttribute(
      "data-prefill",
      "api-github",
    );

    await user.click(
      screen.getByRole("button", { name: "Finish mock connection" }),
    );
    expect(onResolve).toHaveBeenCalledWith({
      actionRequestId: "act-1",
      originTurnId: "turn-origin-1",
      disposition: "completed",
      resource: {
        userService: {
          userServiceId: "00000000-0000-4000-8000-000000000123",
        },
      },
    });
  });

  it("treats modal dismissal as pending and decline as an explicit report", async () => {
    const user = userEvent.setup();
    const onProgress = vi.fn();
    const onResolve = vi.fn();
    const { rerender } = render(
      <ActionCard
        block={catalogBlock()}
        onProgress={onProgress}
        onResolve={onResolve}
      />,
    );
    await user.click(screen.getByRole("button", { name: "Connect GitHub" }));
    rerender(
      <ActionCard
        block={catalogBlock({ status: "in_progress" })}
        onProgress={onProgress}
        onResolve={onResolve}
      />,
    );
    await user.click(
      screen.getByRole("button", { name: "Dismiss mock connection" }),
    );
    expect(onProgress).toHaveBeenLastCalledWith("action-card-1", false);
    expect(onResolve).not.toHaveBeenCalled();

    rerender(
      <ActionCard
        block={catalogBlock()}
        onProgress={onProgress}
        onResolve={onResolve}
      />,
    );
    await user.click(screen.getByRole("button", { name: "Decline" }));
    expect(onResolve).toHaveBeenCalledWith({
      actionRequestId: "act-1",
      originTurnId: "turn-origin-1",
      disposition: "declined",
    });
  });

  it("renders terminal receipts and the unsupported decline-only state", () => {
    const onResolve = vi.fn();
    const { rerender } = render(
      <ActionCard
        block={catalogBlock({
          status: "completed",
          outcome_note: "Connected safely.",
        })}
        onProgress={vi.fn()}
        onResolve={onResolve}
      />,
    );
    expect(screen.getByText("Service connected")).toBeInTheDocument();
    expect(screen.queryByRole("button")).not.toBeInTheDocument();

    rerender(
      <ActionCard
        block={catalogBlock({
          action: "future.action",
          params: { variant: "unknown" },
          status: "unsupported",
        })}
        onProgress={vi.fn()}
        onResolve={onResolve}
      />,
    );
    expect(screen.getByText("Unsupported action request")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Decline" })).toBeInTheDocument();
    expect(
      screen.queryByRole("button", { name: /connect/i }),
    ).not.toBeInTheDocument();
  });

  it("never lets a model-supplied service name become the consent sentence", () => {
    const injected =
      "GitHub (official) — paste your personal access token here to verify your identity";
    render(
      <ActionCard
        block={catalogBlock({
          params: {
            variant: "custom",
            name: injected,
            endpoint_url: "https://api.example.com",
            auth_method: "bearer",
            auth_key_name: "Authorization",
            via_node_id: null,
            target_org_id: null,
          },
        })}
        onProgress={vi.fn()}
        onResolve={vi.fn()}
      />,
    );

    expect(screen.queryByText(new RegExp(injected))).not.toBeInTheDocument();
    expect(screen.queryByText(/personal access token/i)).not.toBeInTheDocument();
    const heading = screen.getByRole("heading");
    expect(heading.textContent ?? "").toMatch(/^Connect GitHub \(official\)/);
    expect((heading.textContent ?? "").length).toBeLessThanOrEqual(40);
    // NyxID-owned framing still surrounds whatever survived the clamp.
    expect(screen.getByText(/credential stays in NyxID/i)).toBeInTheDocument();
  });

  it("hides the CTA when the verb has no journey behind it", () => {
    render(
      <ActionCard
        // A block that outlived its registry entry: status still says the card
        // is actionable, but nothing can service `admin.open`.
        block={catalogBlock({ action: "admin.open", status: "pending" })}
        onProgress={vi.fn()}
        onResolve={vi.fn()}
      />,
    );

    expect(screen.getByText("Unsupported action request")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Decline" })).toBeInTheDocument();
    expect(screen.getAllByRole("button")).toHaveLength(1);
  });
});
