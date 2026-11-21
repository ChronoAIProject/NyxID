//! Pure v1 evidence decoding. No routes, database access, startup work or charging.
pub mod contract;
mod corrections;
mod scalars;
mod strict_json;
mod types;
mod validation;

#[cfg(test)]
mod contract_tests;
#[cfg(test)]
mod correction_tests;
#[cfg(test)]
mod fixture_tests;
#[cfg(test)]
mod resource_tests;
