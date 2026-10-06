//! Release-image verification shared by the server and privileged updater.
#[allow(dead_code)]
#[path = "../../cli/src/tls.rs"]
mod tls;
#[allow(dead_code)]
#[path = "../../cli/src/commands/update_attestation.rs"]
mod update_attestation;

pub mod release_image;
