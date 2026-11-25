//! V1 evidence decoding and opt-in shadow pricing. No wallet/provider charging.
pub mod contract;
mod corrections;
pub mod inbox;
pub mod inbox_authority;
mod inbox_cursor;
mod inbox_facts;
pub mod inbox_promotion;
pub mod inbox_shadow;
mod scalars;
pub mod shadow_buckets;
mod strict_json;
mod types;
mod validation;

#[cfg(test)]
mod chain_tests;
#[cfg(test)]
mod contract_tests;
#[cfg(test)]
mod correction_tests;
#[cfg(test)]
mod fixture_tests;
#[cfg(test)]
mod inbox_tests;

pub mod policy;
pub mod price_pins;
pub mod pricing;
pub mod pricing_schema;
#[cfg(test)]
mod pricing_store_tests;
#[cfg(test)]
mod pricing_tests;
#[cfg(test)]
mod resource_tests;
pub mod tariff_store;

mod completeness_contract;
mod completeness_evidence;
#[cfg(test)]
mod completeness_fixture_tests;
pub mod completeness_inbox;
mod completeness_validation;
mod shadow_completeness;
