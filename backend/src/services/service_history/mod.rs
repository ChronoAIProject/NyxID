//! Transactional service-instance journal. External effects remain with callers.
pub mod context;
pub mod definitions;
pub mod mutation;
mod projection;

pub use mutation::collection;

pub mod relay;

pub mod read;

#[cfg(test)]
mod tests;

pub mod transaction;
