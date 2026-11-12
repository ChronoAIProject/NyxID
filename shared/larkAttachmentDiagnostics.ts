/** Closed native codes only. Never render arbitrary tool/provider error text.
 * Keep aligned with lark_attachment_tool/{diagnostics,eligibility}.rs. */
const reasons = new Set([
  "invalid_arguments", "attachment_unreadable", "attachment_size",
  "attachment_incomplete_set", "attachment_integrity", "field_read_transport",
  "field_read_provider_error", "field_read_invalid_response", "field_missing",
  "field_not_attachment", "field_value_invalid", "upload_transport",
  "upload_provider_error", "upload_invalid_response", "credential_preparation",
  "authority_changed", "admission_refused", "database_unavailable", "internal",
  "ledger_changed", "timeout", "create_provider_error", "create_uncertain",
  ...["uploading", "dispatching", "succeeded", "failed_definite", "failed_pre_dispatch", "uncertain"].map((state) => `ledger:${state}`),
  ...[
    "guest_turn", "flag_off", "not_live_owner_chat", "key_ineligible",
    "catalog_connection_required", "catalog_unavailable", "connection_unavailable",
    "untrusted_spec_override", "catalog_binding_changed", "operation_not_published",
    "required_operation_missing", "untrusted_contract", "contract_changed",
    "invalid_operation_generation", "operation_generation_changed", "hosted_connection_required",
    "connection_credential_required", "credential_unavailable", "credential_changed",
    "destination_changed", "chat_binding_changed", "live_chat_execution_required", "async_unsupported",
  ].map((reason) => `eligibility:${reason}`),
]);

export function larkAttachmentFailureReason(
  tool: string | undefined,
  reason: string | null | undefined,
): string | null {
  const operation = tool?.split("__").at(-1);
  return (operation === "bitable_record_create_with_attachments" || operation === "bitable_media_upload")
    && reason != null && reasons.has(reason) ? reason : null;
}
