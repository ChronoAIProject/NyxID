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
