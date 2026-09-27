#![no_std]

//! Royalty Splitter — a Soroban contract that routes incoming subscription
//! payments to a configurable set of revenue stakeholders.
//!
//! Stakeholders are declared as `(Address, share_ppm)` pairs, where `share_ppm`
//! is a part-per-million weight of the gross payment (`SCALE == 1_000_000`
//! equals 100%). Because percentages such as `33.333%` cannot be expressed as
//! integer basis points, the contract keeps four decimal places of a percent
//! and hands the rounding remainder to the **final** payee, so no fractional
//! "dust" is ever stranded in the contract.
//!
//! Reconfiguration is guarded by a multi-sig proposal: every *current*
//! stakeholder must approve a new split table before it takes effect.

use soroban_sdk::{
    contract, contracterror, contractevent, contractimpl, contracttype, token, Address, Env, Vec,
};

mod distribution;

/// Denominator for split weights: `1_000_000 == 100%`.
///
/// This part-per-million scale resolves splits like `33.333%` exactly
/// (`333_330`), which a basis-point (`10_000`) representation could not.
pub const SCALE: u32 = 1_000_000;

/// Bump the contract instance TTL when it falls below this many ledgers.
const TTL_THRESHOLD: u32 = 100_000;
/// Ledger count the instance TTL is extended to (~30 days at 5s per ledger).
const TTL_EXTEND_TO: u32 = 518_400;

/// A single revenue stakeholder and its weight of every processed payment.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Payee {
    /// Destination of this stakeholder's share.
    pub address: Address,
    /// Weight in parts-per-million; all weights must sum to [`SCALE`].
    pub share_ppm: u32,
}

/// A reconfiguration awaiting multi-sig approval.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PendingConfig {
    /// The split table that is applied once every current stakeholder approves.
    pub splits: Vec<Payee>,
    /// Address that opened the proposal; may cancel it.
    pub proposer: Address,
    /// Current stakeholders that have approved so far.
    pub approvals: Vec<Address>,
}

/// Storage keys for the contract instance.
#[contracttype]
#[derive(Clone)]
pub enum DataKey {
    Admin,
    Token,
    Splits,
    Pending,
}

/// Errors returned by the contract.
#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum Error {
    AlreadyInitialized = 1,
    NotInitialized = 2,
    Unauthorized = 3,
    EmptySplits = 4,
    ZeroShare = 5,
    DuplicatePayee = 6,
    SplitTotalMismatch = 7,
    InvalidAmount = 8,
    Overflow = 9,
    NoPendingConfig = 10,
    AlreadyApproved = 11,
    NotAStakeholder = 12,
    ConfigUnchanged = 13,
}

/// Emitted once when the contract is configured.
#[contractevent]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Initialized {
    #[topic]
    pub admin: Address,
    pub token: Address,
    pub payees: u32,
}

/// Emitted after a payment with the exact per-payee shares that were routed.
#[contractevent]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PaymentProcessed {
    #[topic]
    pub payer: Address,
    pub amount: i128,
    pub shares: Vec<i128>,
}

/// Emitted when a reconfiguration proposal is opened.
#[contractevent]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ConfigProposed {
    #[topic]
    pub proposer: Address,
    pub payees: u32,
}

/// Emitted for each approval; `applied` is true when it activated the config.
#[contractevent]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ConfigApproved {
    #[topic]
    pub approver: Address,
    pub approvals: u32,
    pub applied: bool,
}

/// Emitted when a pending proposal is discarded.
#[contractevent]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ConfigCanceled {
    #[topic]
    pub caller: Address,
    pub approvals: u32,
}

#[contract]
pub struct RoyaltySplitter;

#[contractimpl]
impl RoyaltySplitter {
    /// One-time setup.
    ///
    /// # Arguments
    /// * `admin`  — address allowed to propose reconfigurations and cancel them.
    /// * `token`  — the SEP-41 asset (typically a Stellar Asset Contract) that
    ///   payments are denominated in.
    /// * `splits` — payees whose weights sum to exactly [`SCALE`].
    ///
    /// Requires authorization from `admin`.
    pub fn initialize(
        env: Env,
        admin: Address,
        token: Address,
        splits: Vec<Payee>,
    ) -> Result<(), Error> {
        admin.require_auth();
        bump_instance_ttl(&env);

        let store = env.storage().instance();
        if store.has(&DataKey::Admin) {
            return Err(Error::AlreadyInitialized);
        }
        distribution::validate_splits(&splits)?;

        store.set(&DataKey::Admin, &admin);
        store.set(&DataKey::Token, &token);
        store.set(&DataKey::Splits, &splits);

        Initialized {
            admin,
            token,
            payees: splits.len(),
        }
        .publish(&env);
        Ok(())
    }

    /// Splits `amount` of the configured token from `payer` across all payees.
    ///
    /// Every transfer is executed sequentially against the token contract, and
    /// the returned vector mirrors the configured split order. The final payee
    /// receives the rounding remainder, so the shares always sum to `amount`
    /// and the contract never retains custody of any fraction of the payment.
    ///
    /// Requires authorization from `payer`.
    pub fn process_payment(env: Env, payer: Address, amount: i128) -> Result<Vec<i128>, Error> {
        payer.require_auth();
        if amount <= 0 {
            return Err(Error::InvalidAmount);
        }
        bump_instance_ttl(&env);

        let store = env.storage().instance();
        let token_id: Address = store.get(&DataKey::Token).ok_or(Error::NotInitialized)?;
        let splits: Vec<Payee> = store.get(&DataKey::Splits).ok_or(Error::NotInitialized)?;

        let shares = distribution::compute_shares(&env, &splits, amount)?;
        let token_client = token::TokenClient::new(&env, &token_id);

        let mut i: u32 = 0;
        while i < splits.len() {
            let payee = splits.get(i).ok_or(Error::Overflow)?;
            let share = shares.get(i).ok_or(Error::Overflow)?;
            // Skip zero-value transfers: the token contract rejects a
            // transfer of 0, and a dust-sized payment can legitimately floor
            // a leading share to zero.
            if share > 0 {
                token_client.transfer(&payer, &payee.address, &share);
            }
            i += 1;
        }

        PaymentProcessed {
            payer,
            amount,
            shares: shares.clone(),
        }
        .publish(&env);

        Ok(shares)
    }

    /// Opens a reconfiguration proposal.
    ///
    /// The caller must be the admin or a *current* stakeholder. Nothing changes
    /// until every current stakeholder calls [`Self::approve_splits`].
    pub fn propose_splits(
        env: Env,
        proposer: Address,
        new_splits: Vec<Payee>,
    ) -> Result<(), Error> {
        proposer.require_auth();
        bump_instance_ttl(&env);

        let store = env.storage().instance();
        let admin: Address = store.get(&DataKey::Admin).ok_or(Error::NotInitialized)?;
        let current: Vec<Payee> = store.get(&DataKey::Splits).ok_or(Error::NotInitialized)?;

        distribution::validate_splits(&new_splits)?;

        if new_splits == current {
            return Err(Error::ConfigUnchanged);
        }
        if proposer != admin && !contains_payee(&current, &proposer) {
            return Err(Error::Unauthorized);
        }

        let payees = new_splits.len();
        let pending = PendingConfig {
            splits: new_splits,
            proposer: proposer.clone(),
            approvals: Vec::new(&env),
        };
        store.set(&DataKey::Pending, &pending);

        ConfigProposed { proposer, payees }.publish(&env);
        Ok(())
    }

    /// Records a stakeholder approval and applies the new configuration once
    /// every current stakeholder has approved.
    ///
    /// Returns `true` when this call was the final approval that applied the
    /// pending configuration.
    ///
    /// Approval is scoped to the *current* split table, so stakeholders cannot
    /// be swapped out unilaterally mid-proposal.
    pub fn approve_splits(env: Env, approver: Address) -> Result<bool, Error> {
        approver.require_auth();
        bump_instance_ttl(&env);

        let store = env.storage().instance();
        let current: Vec<Payee> = store.get(&DataKey::Splits).ok_or(Error::NotInitialized)?;
        let mut pending: PendingConfig =
            store.get(&DataKey::Pending).ok_or(Error::NoPendingConfig)?;

        if !contains_payee(&current, &approver) {
            return Err(Error::NotAStakeholder);
        }
        if pending.approvals.contains(&approver) {
            return Err(Error::AlreadyApproved);
        }

        pending.approvals.push_back(approver.clone());
        let applied = pending.approvals.len() == current.len();

        if applied {
            store.set(&DataKey::Splits, &pending.splits);
            store.remove(&DataKey::Pending);
        } else {
            store.set(&DataKey::Pending, &pending);
        }

        ConfigApproved {
            approver,
            approvals: pending.approvals.len(),
            applied,
        }
        .publish(&env);

        Ok(applied)
    }

    /// Cancels a pending proposal. Callable by the proposer or the admin.
    pub fn cancel_splits(env: Env, caller: Address) -> Result<(), Error> {
        caller.require_auth();
        bump_instance_ttl(&env);

        let store = env.storage().instance();
        let admin: Address = store.get(&DataKey::Admin).ok_or(Error::NotInitialized)?;
        let pending: PendingConfig = store.get(&DataKey::Pending).ok_or(Error::NoPendingConfig)?;

        if caller != admin && caller != pending.proposer {
            return Err(Error::Unauthorized);
        }

        store.remove(&DataKey::Pending);
        ConfigCanceled {
            caller,
            approvals: pending.approvals.len(),
        }
        .publish(&env);
        Ok(())
    }

    // ----- Read-only helpers ------------------------------------------------

    /// Returns the active split table.
    pub fn get_splits(env: Env) -> Result<Vec<Payee>, Error> {
        env.storage()
            .instance()
            .get(&DataKey::Splits)
            .ok_or(Error::NotInitialized)
    }

    /// Returns the configured payment token (SEP-41 contract address).
    pub fn get_token(env: Env) -> Result<Address, Error> {
        env.storage()
            .instance()
            .get(&DataKey::Token)
            .ok_or(Error::NotInitialized)
    }

    /// Returns the administrator address.
    pub fn get_admin(env: Env) -> Result<Address, Error> {
        env.storage()
            .instance()
            .get(&DataKey::Admin)
            .ok_or(Error::NotInitialized)
    }

    /// Returns the pending proposal, if any.
    pub fn get_pending(env: Env) -> Option<PendingConfig> {
        env.storage().instance().get(&DataKey::Pending)
    }
}

/// Extends the contract instance TTL so long-lived payment streams and split
/// configurations are not archived while still in use.
fn bump_instance_ttl(env: &Env) {
    env.storage()
        .instance()
        .extend_ttl(TTL_THRESHOLD, TTL_EXTEND_TO);
}

/// Returns `true` when `who` is one of the configured payees.
fn contains_payee(splits: &Vec<Payee>, who: &Address) -> bool {
    let mut i: u32 = 0;
    while i < splits.len() {
        if let Some(payee) = splits.get(i) {
            if &payee.address == who {
                return true;
            }
        }
        i += 1;
    }
    false
}
