#[path = "../build-support/source_identity.rs"]
mod source_identity;

fn main() {
    source_identity::emit();
}
