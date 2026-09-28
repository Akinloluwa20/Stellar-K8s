#![no_std]
//! # Royalty Splitter
//!
//! A dynamic on-chain revenue splitter. It collects an incoming subscription
//! payment and atomically routes it to `N` configured payees according to
//! integer percentage shares, with **no dust left behind**.
//!
//! ## Lifecycle
//!
//! 1. [`RoyaltySplitter::initialize`] — set the founding payee set. Every
//!    founding payee must authorize the call, which prevents an attacker from
//!    front-running deployment with their own recipients.
//! 2. [`RoyaltySplitter::process_payment`] — pull `amount` of an asset from a
//!    payer and fan it out to all payees in a single atomic invocation.
//! 3. [`RoyaltySplitter::update_splits`] — reconfigure the split. This is a
//!    multi-sig operation: it succeeds only if *all current* payees both appear
//!    in the `approvals` list and authorize the invocation, so existing
//!    stakeholders must unanimously consent before the payout targets change.
//!
//! ## Dust safety
//!
//! Splits are expressed in [`distribution::TOTAL_SHARES`] (parts per million).
//! Truncation remainders are paid to the final payee in the list, guaranteeing
//! that the allocations sum to exactly the payment amount — see
//! [`distribution::compute_allocation`].
//!
//! ## Token compatibility
//!
//! The contract talks to assets exclusively through the standard Soroban token
//! interface ([`soroban_sdk::token::Client`]), so it works with Stellar Asset
//! Contracts (SAC) and any token implementing the same interface. See
//! `tests/integration.rs` for end-to-end coverage against a real SAC.

pub mod distribution;

use distribution::{approvals_cover, compute_allocation, validate};
// Re-export the public data types at the crate root so callers and integration
// tests can name them directly.
pub use distribution::{Allocation, Payee, SplitConfig, SplitError};
use soroban_sdk::{contract, contractevent, contractimpl, contracttype, token, Address, Env, Vec};

/// Refresh instance-storage TTL once it drops below this many ledgers.
const CONFIG_TTL_THRESHOLD: u32 = 100;
/// Extend instance storage to this many ledgers whenever it is touched.
const CONFIG_TTL_EXTEND_TO: u32 = 100_000;

/// Storage keys for contract state.
#[contracttype]
#[derive(Clone)]
pub enum DataKey {
    /// Active [`SplitConfig`].
    Config,
    /// Initialization guard.
    Initialized,
    /// Running count of successfully settled payments.
    PaymentsProcessed,
    /// Cumulative amount routed to payees, across all payments.
    TotalRouted,
}

/// Emitted once when the contract is configured.
#[contractevent]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Initialized {
    /// Number of configured payees.
    pub payees: u32,
}

/// Emitted after a payment, carrying the exact amount that was routed.
#[contractevent]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PaymentProcessed {
    /// Payer whose funds were collected.
    #[topic]
    pub from: Address,
    /// Asset that was transferred.
    pub asset: Address,
    /// Gross amount collected.
    pub amount: i128,
    /// Total routed to payees (always equal to `amount`).
    pub routed: i128,
}

/// Emitted when the split table changes.
#[contractevent]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SplitUpdated {
    /// The newly active payee set.
    pub payees: Vec<Payee>,
}

/// The royalty splitter contract.
#[contract]
pub struct RoyaltySplitter;

#[contractimpl]
impl RoyaltySplitter {
    /// Configure the founding payee set.
    ///
    /// Can only be called once. Every listed payee must authorize the call so
    /// the initial split cannot be hijacked between deployment and use.
    ///
    /// # Errors
    /// * [`SplitError::AlreadyInitialized`] if called twice.
    /// * Any validation error from [`distribution::validate`].
    pub fn initialize(env: Env, payees: Vec<Payee>) -> Result<(), SplitError> {
        if env.storage().instance().has(&DataKey::Initialized) {
            return Err(SplitError::AlreadyInitialized);
        }
        validate(&payees)?;

        // Require consent from every founding payee.
        for payee in payees.iter() {
            payee.address.require_auth();
        }

        let payee_count = payees.len();
        env.storage()
            .instance()
            .set(&DataKey::Config, &SplitConfig { payees });
        env.storage().instance().set(&DataKey::Initialized, &true);
        env.storage()
            .instance()
            .set(&DataKey::PaymentsProcessed, &0u64);
        env.storage().instance().set(&DataKey::TotalRouted, &0i128);
        bump_ttl(&env);

        Initialized {
            payees: payee_count,
        }
        .publish(&env);
        Ok(())
    }

    /// Collect `amount` of `asset` from `from` and route it to every payee.
    ///
    /// The operation is atomic: the payment is pulled into the contract and
    /// then fanned out in the same invocation. If any transfer fails the whole
    /// call reverts, so funds can never be stuck mid-distribution. The contract
    /// retains a zero balance once the call returns.
    ///
    /// # Errors
    /// * [`SplitError::NotInitialized`] before the split is configured.
    /// * [`SplitError::InvalidAmount`] for a non-positive amount.
    /// * [`SplitError::MathOverflow`] if the split math overflows `i128`.
    pub fn process_payment(
        env: Env,
        from: Address,
        asset: Address,
        amount: i128,
    ) -> Result<(), SplitError> {
        // Only the payer may move their own funds.
        from.require_auth();

        let config = load_config(&env)?;
        let allocations = compute_allocation(&env, amount, &config.payees)?;

        // Use the canonical token interface so any SAC-compatible asset works.
        let asset_client = token::Client::new(&env, &asset);
        let contract = env.current_contract_address();

        // Pull the payment in first; everything below settles it atomically.
        asset_client.transfer(&from, &contract, &amount);

        let mut routed: i128 = 0;
        for allocation in allocations.iter() {
            asset_client.transfer(&contract, &allocation.address, &allocation.amount);
            // Cannot overflow: allocations are validated to sum to `amount`.
            routed += allocation.amount;
        }

        // Observability: counters let indexers and dashboards track the stream
        // without replaying every transfer event.
        let processed: u64 = env
            .storage()
            .instance()
            .get(&DataKey::PaymentsProcessed)
            .unwrap_or(0);
        env.storage()
            .instance()
            .set(&DataKey::PaymentsProcessed, &(processed + 1));

        let total: i128 = env
            .storage()
            .instance()
            .get(&DataKey::TotalRouted)
            .unwrap_or(0);
        env.storage()
            .instance()
            .set(&DataKey::TotalRouted, &(total + routed));

        PaymentProcessed {
            from,
            asset,
            amount,
            routed,
        }
        .publish(&env);
        bump_ttl(&env);
        Ok(())
    }

    /// Reconfigure the split under unanimous multi-sig approval.
    ///
    /// The `approvals` list must name **every current payee** and each of those
    /// payees must have authorized this invocation. Together this gives the
    /// existing stakeholders a collective veto: no change to who gets paid can
    /// happen unless all of them sign the same call.
    ///
    /// # Errors
    /// * [`SplitError::NotInitialized`] before the split is configured.
    /// * [`SplitError::MissingApproval`] if any current payee is absent or has
    ///   not authorized the call.
    /// * Any validation error from [`distribution::validate`] for `new_payees`.
    pub fn update_splits(
        env: Env,
        new_payees: Vec<Payee>,
        approvals: Vec<Address>,
    ) -> Result<(), SplitError> {
        let current = load_config(&env)?;

        // Multi-sig gate: approvals must cover the current payee set exactly.
        if !approvals_cover(&current.payees, &approvals) {
            return Err(SplitError::MissingApproval);
        }
        for payee in current.payees.iter() {
            payee.address.require_auth();
        }

        validate(&new_payees)?;

        env.storage().instance().set(
            &DataKey::Config,
            &SplitConfig {
                payees: new_payees.clone(),
            },
        );
        bump_ttl(&env);

        SplitUpdated { payees: new_payees }.publish(&env);
        Ok(())
    }

    /// Return the currently active split.
    pub fn get_config(env: Env) -> Result<SplitConfig, SplitError> {
        load_config(&env)
    }

    /// Simulate a payment: resolve `amount` into exact per-payee allocations
    /// without moving any funds. Useful for off-chain callers and for asserting
    /// dust-free behaviour before submitting a transaction.
    pub fn preview(env: Env, amount: i128) -> Result<Vec<Allocation>, SplitError> {
        let config = load_config(&env)?;
        compute_allocation(&env, amount, &config.payees)
    }

    /// Number of payments successfully settled by this contract.
    pub fn payments_processed(env: Env) -> u64 {
        env.storage()
            .instance()
            .get(&DataKey::PaymentsProcessed)
            .unwrap_or(0)
    }

    /// Cumulative amount routed to payees across all payments.
    pub fn total_routed(env: Env) -> i128 {
        env.storage()
            .instance()
            .get(&DataKey::TotalRouted)
            .unwrap_or(0)
    }
}

/// Load the active configuration or report that the contract is uninitialized.
fn load_config(env: &Env) -> Result<SplitConfig, SplitError> {
    env.storage()
        .instance()
        .get(&DataKey::Config)
        .ok_or(SplitError::NotInitialized)
}

/// Keep instance storage (which holds the split config) from being archived.
fn bump_ttl(env: &Env) {
    env.storage()
        .instance()
        .extend_ttl(CONFIG_TTL_THRESHOLD, CONFIG_TTL_EXTEND_TO);
}
