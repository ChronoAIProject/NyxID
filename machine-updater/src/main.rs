//! Dedicated companion binary: no shell, node credentials or arbitrary commands.
#[path = "../../cli/src/machine_updater/mod.rs"]
mod machine_updater;
#[allow(dead_code)]
#[path = "../../cli/src/tls.rs"]
mod tls;
#[allow(dead_code)] // Shared verifier also exposes the regular CLI-release entry point.
#[path = "../../cli/src/commands/update_attestation.rs"]
mod update_attestation;

#[tokio::main]
async fn main() {
    let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
    let args: Vec<String> = std::env::args().skip(1).collect();
    let root = nyxid_machine::update::UPDATE_VOLUME.into();
    let result = match args.as_slice() {
        [action, version] if action == "verify" => {
            machine_updater::verify(root, version.clone()).await
        }
        [action, name] if action == "watch" => machine_updater::run(root, name.clone()).await,
        [action, name, version] if action == "bootstrap" => {
            machine_updater::bootstrap(root, name.clone(), version.clone()).await
        }
        _ => Err(anyhow::anyhow!(
            "usage: nyxid-machine-updater watch NAME | bootstrap NAME VERSION | verify VERSION"
        )),
    };
    if let Err(error) = result {
        // Neither Docker metadata nor environment/credentials appear in errors.
        eprintln!("{}", machine_updater::Failure::classify(&error, "startup"));
        std::process::exit(1);
    }
}
