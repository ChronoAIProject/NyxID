import type {
  ActionCardContentBlock,
  ApprovalCardContentBlock,
} from "@/types/assistant";

export interface ActionCardGallerySpecimen {
  readonly caption: string;
  readonly block: ActionCardContentBlock;
}

export interface ApprovalCardGallerySpecimen {
  readonly caption: string;
  readonly block: ApprovalCardContentBlock;
}

const githubParams: ActionCardContentBlock["params"] = {
  variant: "catalog",
  service_slug: "api-github",
  requested_scopes: ["repo"],
  via_node_id: null,
  target_org_id: null,
};

function githubAction(
  status: ActionCardContentBlock["status"],
  outcomeNote = "",
): ActionCardContentBlock {
  return {
    type: "action_card",
    block_id: `gallery-service-connect-${status}`,
    action: "service.connect",
    action_request_id: "req_7",
    origin_turn_id: "turn_7",
    params: githubParams,
    status,
    outcome_note: outcomeNote,
  };
}

const actionCards: readonly ActionCardGallerySpecimen[] = [
  {
    caption: "state: pending",
    block: githubAction("pending"),
  },
  {
    caption: "state: in_progress",
    block: githubAction("in_progress"),
  },
  {
    caption: "state: completed",
    block: githubAction(
      "completed",
      "GitHub is available to the assistant through service us_44.",
    ),
  },
  {
    caption: "state: declined",
    block: githubAction(
      "declined",
      "GitHub was not connected. No account access was granted.",
    ),
  },
  {
    caption: "state: failed",
    block: githubAction(
      "failed",
      "GitHub could not be connected. No account access was granted.",
    ),
  },
  {
    caption: "state: unsupported",
    block: {
      ...githubAction("unsupported"),
      action: "future.action",
      params: { variant: "unknown" },
    },
  },
  {
    caption: "variant: custom endpoint",
    block: {
      ...githubAction("pending"),
      block_id: "gallery-service-connect-custom",
      params: {
        variant: "custom",
        name: "Internal API",
        endpoint_url: "https://api.internal.example.com/v1",
        auth_method: "bearer",
        auth_key_name: "X-Api-Key",
        via_node_id: null,
        target_org_id: null,
      },
    },
  },
  {
    caption: "variant: via node + organization",
    block: {
      ...githubAction("pending"),
      block_id: "gallery-service-connect-routed",
      params: {
        ...githubParams,
        via_node_id: "n_77",
        target_org_id: "Current organisation",
      },
    },
  },
];

function approval(
  caption: string,
  overrides: Partial<ApprovalCardContentBlock>,
): ApprovalCardGallerySpecimen {
  return {
    caption,
    block: {
      type: "approval_card",
      block_id: `gallery-${caption.replaceAll(" ", "-")}`,
      approval_request_id: "req_7",
      body: "Post the deployment summary to the engineering channel.",
      service_slug: "lark-bot",
      agent_key_prefix: "k_1a2b",
      approval_mode: "per_request",
      grant_duration_sec: null,
      expires_at: new Date(Date.now() + 14 * 60_000).toISOString(),
      decision: null,
      decision_channel: null,
      ...overrides,
    },
  };
}

const approvalCards: readonly ApprovalCardGallerySpecimen[] = [
  approval("state: pending per_request", {}),
  approval("state: pending grant", {
    body: "Create a GitHub issue when a production deployment fails.",
    service_slug: "github",
    approval_mode: "grant",
    grant_duration_sec: 3_600,
  }),
  approval("state: approved", {
    body: "Posted the deployment summary to the engineering channel.",
    decision: "approved",
    decision_channel: "web",
  }),
  approval("state: denied", {
    body: "The requested GitHub write was not sent.",
    service_slug: "github",
    decision: "denied",
    decision_channel: "mobile",
  }),
  approval("state: expired", {
    body: "The deployment summary was not sent before the review expired.",
    decision: "expired",
    decision_channel: null,
  }),
  approval("state: cancelled", {
    body: "The release workflow cancelled this request before review.",
    service_slug: "github",
    decision: "cancelled",
    decision_channel: "telegram",
  }),
];

export const actionCardGalleryFixtures = {
  actionCards,
  approvalCards,
  liveWizard: {
    caption: "live: GitHub managed OAuth",
    block: {
      ...githubAction("pending"),
      block_id: "gallery-live-service-connect",
    },
  } satisfies ActionCardGallerySpecimen,
} as const;
