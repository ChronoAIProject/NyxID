//! Release-image verification shared by the server and privileged updater.
#[allow(dead_code)]
#[path = "../../cli/src/tls.rs"]
mod tls;
#[allow(dead_code)]
#[path = "../../cli/src/commands/update_attestation.rs"]
mod update_attestation;

pub mod release_image;
/// Generic provider protocol/deadline journal; requires a selected privileged
/// runtime adapter and is not itself a physical enforcement implementation.
pub mod runtime_provider;
/// Thin, manager-owned adapter for the Agent Sandbox Kubernetes API.
///
/// This module intentionally contains only Sandbox CR submission/observation
/// primitives. It never creates Pods or PVCs and it does not execute commands
/// through Kubernetes. The Agent Sandbox controller remains the sole
/// reconciler for those resources.
pub mod sandbox;

/// Backend-only Kubernetes request credentials, distinct from Pod identity.
pub mod kubernetes_credentials;
