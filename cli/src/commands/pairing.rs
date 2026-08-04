//! `nyxid pairing` — management commands for remote CLI pairings.
//!
//! Today this has one subcommand, `resume`, used with the `--no-wait`
//! flag on the wizard-capable commands: the initial invocation creates
//! a pairing and exits after printing the code + URL; the user (or the
//! AI agent relaying on their behalf) later runs `nyxid pairing resume
//! <ID>` to pick up the completed state. The resume path dispatches to
//! the same kind-aware printer the interactive pairing flow uses so
//! scripts don't care which transport produced the record.

use anyhow::Result;
use serde_json::{Value, json};

use crate::cli::{OutputFormat, PairingCommands};
use crate::wizard::{self, pairing};

pub async fn run(command: PairingCommands) -> Result<()> {
    match command {
        PairingCommands::Resume { id, auth } => {
            let base_url = auth.resolved_base_url()?;
            let output = auth.output;
            // Peek at the pairing's server-reported kind before the
            // outcome loses that info — both api-key rotate and node
            // rotate-token yield `RotationAcknowledged`, which is
            // kind-ambiguous. The poll response still has the exact
            // `PairingFlow` so we can preserve it in the JSON
            // payload for agent automation.
            let outcome = pairing::resume_pairing(&auth, &id).await?;

            match output {
                OutputFormat::Json => {
                    // Agent-oriented: emit the outcome as a
                    // machine-parseable JSON object on stdout so
                    // wrappers can recover the created/rotated id
                    // after a `--no-wait` handoff. The ack bodies
                    // themselves use narrow `deny_unknown_fields`
                    // shapes (see `cli/src/wizard/mod.rs`) so no
                    // secret material can slip through.
                    //
                    // For rotation flows we fetch the kind string
                    // separately via `resume_kind` so the JSON
                    // carries the exact workflow (`api-key-rotate`
                    // vs `node-rotate-token`) instead of the
                    // ambiguous `"rotation"` label.
                    let kind_hint = pairing::resume_kind(&auth, &id).await.ok();
                    let payload = outcome_to_json(&id, &outcome, kind_hint.as_deref());
                    println!("{}", serde_json::to_string_pretty(&payload)?);
                }
                OutputFormat::Table => {
                    wizard::print_resume_summary(&auth, &outcome, &base_url).await;
                }
            }

            // Non-zero exit for non-success outcomes so scripts can
            // detect "user cancelled / expired" vs "succeeded"
            // without parsing stderr or the JSON payload.
            if matches!(
                outcome,
                wizard::WizardOutcome::Cancelled | wizard::WizardOutcome::TimedOut
            ) {
                std::process::exit(1);
            }
            Ok(())
        }
    }
}

/// Serialize a resume outcome into a JSON payload for
/// `--output json`. Every variant produces a top-level `status`
/// field plus the non-secret identifiers the CLI summary would
/// have printed. Kept flat so typical `jq` pipelines need no
/// per-kind dispatch.
fn outcome_to_json(
    pairing_id: &str,
    outcome: &wizard::WizardOutcome,
    kind_hint: Option<&str>,
) -> Value {
    match outcome {
        wizard::WizardOutcome::AiKeyPaired(ack) => json!({
            "status": "completed",
            "kind": "ai-key",
            "pairing_id": pairing_id,
            "service_id": ack.service_id,
            "slug": ack.slug,
            "label": ack.label,
        }),
        wizard::WizardOutcome::ApiKeyCreateAcknowledged(ack) => json!({
            "status": "completed",
            "kind": "api-key-create",
            "pairing_id": pairing_id,
            "api_key_id": ack.api_key_id,
        }),
        wizard::WizardOutcome::ServiceAccountCreateAcknowledged(ack) => json!({
            "status": "completed",
            "kind": "service-account-create",
            "pairing_id": pairing_id,
            "service_account_id": ack.service_account_id,
        }),
        wizard::WizardOutcome::DeveloperAppCreateAcknowledged(ack) => json!({
            "status": "completed",
            "kind": "developer-app-create",
            "pairing_id": pairing_id,
            "developer_app_id": ack.developer_app_id,
        }),
        wizard::WizardOutcome::MfaSetupAcknowledged(ack) => json!({
            "status": "completed",
            "kind": "mfa-setup",
            "pairing_id": pairing_id,
            "factor_id": ack.factor_id,
        }),
        wizard::WizardOutcome::NodeRegisterAcknowledged(ack) => json!({
            "status": "completed",
            "kind": "node-register-token",
            "pairing_id": pairing_id,
            "token_id": ack.token_id,
        }),
        wizard::WizardOutcome::RotationAcknowledged(ack) => {
            // `RotationAcknowledged` is produced by all four
            // rotation-shaped flows. Use the kind string the
            // server told us at poll-time so agents can dispatch
            // without digging into the resource id format.
            // Fallback to "rotation" when the server returned an
            // unknown kind or the poll call failed — less precise
            // but still accurate at a higher level.
            let kind = kind_hint
                .filter(|k| {
                    matches!(
                        *k,
                        "api-key-rotate"
                            | "node-rotate-token"
                            | "service-account-rotate-secret"
                            | "developer-app-rotate-secret"
                    )
                })
                .unwrap_or("rotation");
            json!({
                "status": "completed",
                "kind": kind,
                "pairing_id": pairing_id,
                "resource_id": ack.resource_id,
            })
        }
        wizard::WizardOutcome::AiKeyCompleted(_) => json!({
            "status": "completed",
            "kind": "ai-key-local",
            "pairing_id": pairing_id,
        }),
        wizard::WizardOutcome::Cancelled => json!({
            "status": "cancelled",
            "pairing_id": pairing_id,
        }),
        wizard::WizardOutcome::TimedOut => json!({
            "status": "expired",
            "pairing_id": pairing_id,
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // The status strings here are load-bearing: `run` exits non-zero on
    // cancelled/expired so scripts can detect outcome without parsing.
    #[test]
    fn cancelled_outcome_maps_to_cancelled_status() {
        let v = outcome_to_json("pair-1", &wizard::WizardOutcome::Cancelled, None);
        assert_eq!(v["status"], "cancelled");
        assert_eq!(v["pairing_id"], "pair-1");
    }

    #[test]
    fn timed_out_outcome_maps_to_expired_status() {
        let v = outcome_to_json("pair-2", &wizard::WizardOutcome::TimedOut, None);
        assert_eq!(v["status"], "expired");
        assert_eq!(v["pairing_id"], "pair-2");
    }

    #[test]
    fn ai_key_paired_maps_to_ai_key_kind_with_identifiers() {
        let outcome = wizard::WizardOutcome::AiKeyPaired(wizard::AiKeyPairingAckPayload {
            acknowledged: true,
            service_id: "svc-1".to_string(),
            slug: "openai".to_string(),
            label: "OpenAI".to_string(),
        });
        let v = outcome_to_json("pair-3", &outcome, None);
        assert_eq!(v["status"], "completed");
        assert_eq!(v["kind"], "ai-key");
        assert_eq!(v["pairing_id"], "pair-3");
        assert_eq!(v["service_id"], "svc-1");
        assert_eq!(v["slug"], "openai");
        assert_eq!(v["label"], "OpenAI");
    }

    #[test]
    fn api_key_create_maps_to_api_key_create_kind() {
        let outcome =
            wizard::WizardOutcome::ApiKeyCreateAcknowledged(wizard::ApiKeyCreateAckPayload {
                acknowledged: true,
                api_key_id: "ak-1".to_string(),
            });
        let v = outcome_to_json("pair-4", &outcome, None);
        assert_eq!(v["status"], "completed");
        assert_eq!(v["kind"], "api-key-create");
        assert_eq!(v["api_key_id"], "ak-1");
    }

    #[test]
    fn service_account_create_maps_to_service_account_create_kind() {
        let outcome = wizard::WizardOutcome::ServiceAccountCreateAcknowledged(
            wizard::ServiceAccountCreateAckPayload {
                acknowledged: true,
                service_account_id: "sa-1".to_string(),
            },
        );
        let v = outcome_to_json("pair-5", &outcome, None);
        assert_eq!(v["kind"], "service-account-create");
        assert_eq!(v["service_account_id"], "sa-1");
    }

    #[test]
    fn developer_app_create_maps_to_developer_app_create_kind() {
        let outcome = wizard::WizardOutcome::DeveloperAppCreateAcknowledged(
            wizard::DeveloperAppCreateAckPayload {
                acknowledged: true,
                developer_app_id: "da-1".to_string(),
            },
        );
        let v = outcome_to_json("pair-6", &outcome, None);
        assert_eq!(v["kind"], "developer-app-create");
        assert_eq!(v["developer_app_id"], "da-1");
    }

    #[test]
    fn mfa_setup_maps_to_mfa_setup_kind() {
        let outcome = wizard::WizardOutcome::MfaSetupAcknowledged(wizard::MfaSetupAckPayload {
            acknowledged: true,
            factor_id: "factor-1".to_string(),
        });
        let v = outcome_to_json("pair-7", &outcome, None);
        assert_eq!(v["kind"], "mfa-setup");
        assert_eq!(v["factor_id"], "factor-1");
    }

    #[test]
    fn node_register_maps_to_node_register_token_kind() {
        let outcome =
            wizard::WizardOutcome::NodeRegisterAcknowledged(wizard::NodeRegisterAckPayload {
                acknowledged: true,
                token_id: "tok-1".to_string(),
            });
        let v = outcome_to_json("pair-8", &outcome, None);
        assert_eq!(v["kind"], "node-register-token");
        assert_eq!(v["token_id"], "tok-1");
    }

    #[test]
    fn ai_key_completed_maps_to_ai_key_local_kind() {
        let outcome = wizard::WizardOutcome::AiKeyCompleted(serde_json::json!({"ignored": true}));
        let v = outcome_to_json("pair-9", &outcome, None);
        assert_eq!(v["status"], "completed");
        assert_eq!(v["kind"], "ai-key-local");
        assert_eq!(v["pairing_id"], "pair-9");
    }

    fn rotation(resource_id: &str) -> wizard::WizardOutcome {
        wizard::WizardOutcome::RotationAcknowledged(wizard::RotationAckPayload {
            acknowledged: true,
            resource_id: resource_id.to_string(),
        })
    }

    #[test]
    fn rotation_preserves_valid_kind_hint() {
        // Each of the four rotation-shaped flows must round-trip its
        // exact kind so agents can dispatch without parsing resource ids.
        for kind in [
            "api-key-rotate",
            "node-rotate-token",
            "service-account-rotate-secret",
            "developer-app-rotate-secret",
        ] {
            let v = outcome_to_json("pair-r", &rotation("res-1"), Some(kind));
            assert_eq!(v["status"], "completed");
            assert_eq!(v["kind"], kind, "kind hint {kind} should pass through");
            assert_eq!(v["resource_id"], "res-1");
        }
    }

    #[test]
    fn rotation_falls_back_to_rotation_on_unknown_kind() {
        let v = outcome_to_json("pair-r", &rotation("res-2"), Some("bogus-kind"));
        assert_eq!(v["kind"], "rotation");
        assert_eq!(v["resource_id"], "res-2");
    }

    #[test]
    fn rotation_falls_back_to_rotation_when_kind_hint_absent() {
        let v = outcome_to_json("pair-r", &rotation("res-3"), None);
        assert_eq!(v["kind"], "rotation");
        assert_eq!(v["resource_id"], "res-3");
    }
}
