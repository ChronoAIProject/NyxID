export type OwnershipResourceKind = "service" | "channel_bot";

export interface OwnershipResource {
  readonly id: string;
  readonly name: string;
  readonly owner_user_id: string;
  readonly platform: string | null;
  readonly slug: string | null;
}

export interface OwnershipTransferPreview {
  readonly resource_kind: OwnershipResourceKind;
  readonly resource_id: string;
  readonly name: string;
  readonly previous_owner_user_id: string;
  readonly previous_owner_name?: string;
  readonly new_owner_user_id: string;
  readonly destination_name: string;
  readonly destination_type: "person" | "org";
  readonly version: string;
  readonly routes_to_retire: number;
  readonly blockers: readonly string[];
  readonly effects: readonly string[];
}

export interface OwnershipTransferRequest {
  readonly new_owner_user_id: string;
  readonly request_id: string;
  readonly expected_version: string;
}

export interface OwnershipTransferResult {
  readonly transfer_id: string;
  readonly resource_kind: OwnershipResourceKind;
  readonly resource_id: string;
  readonly previous_owner_user_id: string;
  readonly new_owner_user_id: string;
  readonly retired_routes: number;
}

export interface OwnershipResourceList {
  readonly items: readonly OwnershipResource[];
  readonly next_offset: number | null;
}

export interface OwnershipDestination {
  readonly id: string;
  readonly display_name: string | null;
  readonly email: string;
  readonly is_active: boolean;
}
