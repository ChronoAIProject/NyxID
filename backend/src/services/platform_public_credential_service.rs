//! Public execution credential producer. An unverified runtime has no issuer.
//! Owner credential convergence, history and recovery are never used here.
use super::{platform_manager_contract::*, platform_public_policy};
use crate::{AppState, errors::AppResult};
use async_trait::async_trait;
use mongodb::Database;
use std::sync::Arc;

pub fn producer(_state: AppState) -> Arc<dyn PublicNyxAgentRuntime> {
    Arc::new(UnverifiedPublicRuntime)
}
struct UnverifiedPublicRuntime;

#[async_trait]
impl PublicNyxAgentRuntime for UnverifiedPublicRuntime {
    async fn prepare(
        &self,
        _db: &Database,
        _context: &PublicExecutionContext,
        _input: TurnInput,
    ) -> AppResult<PreparedPublicTurn> {
        Err(platform_public_policy::unavailable())
    }
    async fn replace_credential(
        &self,
        _db: &Database,
        _context: &PublicExecutionContext,
        _expected_key_id: &str,
    ) -> AppResult<RuntimeCredential> {
        Err(platform_public_policy::unavailable())
    }
    async fn record_upstream_binding(
        &self,
        _db: &Database,
        _context: &PublicExecutionContext,
        _credential_api_key_id: &str,
        _binding: &ObservedUpstreamBinding,
    ) -> AppResult<()> {
        Err(platform_public_policy::unavailable())
    }
    async fn settle(
        &self,
        _db: &Database,
        _context: &PublicExecutionContext,
        _message_id: &str,
        _result: &super::assistant_nyxagent::TurnResult,
    ) -> AppResult<SettledExecution> {
        // No public turn can be prepared or dispatched in this implementation;
        // never claim a saved reply or converge an owner credential as fallback.
        Err(platform_public_policy::unavailable())
    }
}
