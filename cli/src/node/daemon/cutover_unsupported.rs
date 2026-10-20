use crate::node::error::{Error, Result};
use crate::node::storage::Directory;
use std::path::Path;

#[derive(Clone, Copy, Default)]
pub(crate) struct Cutover<'a> {
    pub(crate) profile: Option<&'a str>,
    pub(crate) confirm_legacy_cutover: bool,
}
pub(crate) struct Quiescence;
impl Quiescence {
    pub(crate) fn check(&mut self) -> Result<()> {
        Err(Error::Config("node_migration_platform_unsupported: use a supported macOS or Linux credential supervisor; retain original custody".into()))
    }
}
pub(crate) fn require_migration_quiescence(
    directory: &Directory,
    config_dir: &Path,
    options: Cutover<'_>,
) -> Result<Quiescence> {
    let _ = (
        directory,
        config_dir,
        options.profile,
        options.confirm_legacy_cutover,
    );
    Quiescence.check()?;
    Ok(Quiescence)
}
