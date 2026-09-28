//! # Royalty Splitter Contract
//!
//! A dynamic royalty splitter for content platforms: incoming subscription
//! payments are automatically routed to N payees according to configurable
//! percentage splits, without ever stranding fractional token dust.
//!
//! ## Design
//!
//! - **Splits** are stored as basis points (1 bp = 0.01%) and must sum to
//!   exactly 10,000 (100%).
//! - **[`RoyaltySplitter::process_payment`]** pulls the full payment amount
//!   from the payer using the token contract, then executes sequential
//!   transfers to every payee. The fractional remainder from truncating each
//!   percentage is assigned to the final payee in the list, so the contract
//!   never holds a balance after a successful payment.
//! - **Reconfiguration** requires multi-sig approval from *all* current
//!   stakeholders: each payee signs the exact proposal with their registered
//!   ed25519 key, and the new configuration is committed atomically once the
//!   last required signature is recorded.
//!
//! ## Authorization
//!
//! - `process_payment` transfers funds from the payer, so the payer (or an
//!   approved subscription-operator contract on their behalf) must authorize
//!   the token transfer.
//! - Proposing or cancelling a reconfiguration requires Soroban
//!   authorization from a current stakeholder.
//! - Approving a reconfiguration verifies an ed25519 signature over the
//!   domain-separated proposal message.
#![no_std]
extern crate alloc;

use soroban_sdk::{
    contract, contracterror, contractimpl, contracttype, Address, Bytes, BytesN, Env, Vec,
};

mod distribution;
mod test;

pub use distribution::{calculate_payouts, validate_total, MAX_TOTAL_BPS};

/// Upper bound on the number of payees, keeping the duplicate-detection and
/// payout loops comfortably within Soroban's CPU instruction budget.
pub const MAX_PAYEES: u32 = 100;

/// Storage keys for the contract's instance data.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DataKey {
    /// The token contract subscription payments are received in.
    Token,
    /// Registered payees in order; the last entry receives all dust.
    Payees,
    /// Basis-point splits, index-aligned with `Payees`.
    Splits,
    /// Ed25519 public keys for each payee (verify reconfig approvals).
    PayeeKeys,
    /// A pending reconfiguration awaiting the remaining approvals.
    PendingConfig,
    /// Stakeholders that have already approved the pending configuration.
    Approvals,
}

/// A pending reconfiguration: the proposed payee list plus its splits.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProposedConfig {
    pub payees: Vec<Address>,
    pub splits: Vec<i128>,
    /// Ed25519 public keys of the *proposed* payees, aligned with `payees`.
    pub payee_keys: Vec<BytesN<32>>,
}

/// Errors raised by the royalty splitter.
#[contracterror]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RoyaltyError {
    /// The contract has not been initialized yet.
    NotInitialized = 1,
    /// Initialization was attempted more than once.
    AlreadyInitialized = 2,
    /// Payee/split/key counts differ, are empty, or exceed `MAX_PAYEES`.
    InvalidPayees = 3,
    /// A split is negative or exceeds 100%.
    SplitOutOfRange = 4,
    /// Splits do not sum to exactly 100%.
    SplitSumInvalid = 5,
    /// A payee address appears more than once.
    DuplicatePayee = 6,
    /// A registered ed25519 key is already bound to another payee.
    DuplicateKey = 7,
    /// A pending configuration exists; commit it or cancel it first.
    PendingConfigExists = 8,
    /// No pending configuration has been proposed.
    NoPendingConfig = 9,
    /// The caller is not a current stakeholder (payee) of the contract.
    NotAStakeholder = 10,
    /// This stakeholder already approved the pending configuration.
    AlreadyApproved = 11,
    /// The ed25519 signature over the proposal is invalid.
    BadSignature = 12,
    /// A computed payout is not a valid token amount.
    InvalidPayout = 13,
    /// The payment amount must be positive.
    InvalidAmount = 14,
    /// The transferred amount did not match the declared payment amount.
    AmountMismatch = 15,
}

/// Reads the configured payees. Empty if uninitialized.
fn get_payees(e: &Env) -> Vec<Address> {
    e.storage()
        .instance()
        .get(&DataKey::Payees)
        .unwrap_or_else(|| Vec::new(e))
}

/// Reads the configured splits, index-aligned with the payees.
fn get_splits(e: &Env) -> Vec<i128> {
    e.storage()
        .instance()
        .get(&DataKey::Splits)
        .unwrap_or_else(|| Vec::new(e))
}

/// Reads the ed25519 public key registered for each payee.
fn get_payee_keys(e: &Env) -> Vec<BytesN<32>> {
    e.storage()
        .instance()
        .get(&DataKey::PayeeKeys)
        .unwrap_or_else(|| Vec::new(e))
}

/// Returns the position of `payee` in `payees`, if present.
fn find_payee(payees: &Vec<Address>, payee: &Address) -> Option<u32> {
    (0..payees.len()).find(|&i| payees.get(i).as_ref() == Some(payee))
}

/// Shared validation for a (payees, splits, keys) configuration.
fn validate_config(
    payees: &Vec<Address>,
    splits: &Vec<i128>,
    payee_keys: &Vec<BytesN<32>>,
) -> Result<(), RoyaltyError> {
    let n = payees.len();
    if n == 0 || n > MAX_PAYEES || n != splits.len() || n != payee_keys.len() {
        return Err(RoyaltyError::InvalidPayees);
    }
    let mut bps_total: i128 = 0;
    for i in 0..n {
        let bps = splits.get(i).unwrap_or(0);
        if !distribution::validate_bps(bps) {
            return Err(RoyaltyError::SplitOutOfRange);
        }
        bps_total += bps;
    }
    if bps_total != MAX_TOTAL_BPS {
        return Err(RoyaltyError::SplitSumInvalid);
    }
    for i in 0..n {
        for j in (i + 1)..n {
            if payees.get(i) == payees.get(j) {
                return Err(RoyaltyError::DuplicatePayee);
            }
            if payee_keys.get(i) == payee_keys.get(j) {
                return Err(RoyaltyError::DuplicateKey);
            }
        }
    }
    Ok(())
}

/// Computes the payout vector for `amount` under the current configuration.
fn compute_payouts(e: &Env, amount: i128) -> Result<Vec<i128>, RoyaltyError> {
    let payees = get_payees(e);
    if payees.is_empty() {
        return Err(RoyaltyError::NotInitialized);
    }
    let splits = get_splits(e);
    let splits_native: alloc::vec::Vec<i128> = splits.iter().collect();
    let mut payouts = Vec::new(e);
    for p in distribution::calculate_payouts(amount, &splits_native) {
        payouts.push_back(p);
    }
    Ok(payouts)
}

/// Builds the domain-separated message that stakeholders sign to approve a
/// reconfiguration proposal. The message binds the proposal to this contract
/// instance (via its strkey) and to the exact payee/split/key tuples, so an
/// approval captured for one proposal or deployment cannot be replayed
/// against another.
fn approval_message(e: &Env, proposal: &ProposedConfig) -> Bytes {
    let mut msg = Bytes::new(e);
    msg.extend_from_slice(b"royalty-splitter:v1:reconfig");
    let contract_id = e.current_contract_address().to_string();
    let mut id_buf = alloc::vec![0u8; contract_id.len() as usize];
    contract_id.copy_into_slice(&mut id_buf);
    msg.extend_from_slice(&id_buf);
    for i in 0..proposal.payees.len() {
        // Validation guarantees index i exists for all three lists.
        let payee_str = proposal.payees.get_unchecked(i).to_string();
        let mut payee_buf = alloc::vec![0u8; payee_str.len() as usize];
        payee_str.copy_into_slice(&mut payee_buf);
        msg.extend_from_slice(&payee_buf);
        msg.extend_from_slice(&proposal.splits.get_unchecked(i).to_be_bytes());
        let mut key_buf = [0u8; 32];
        proposal
            .payee_keys
            .get_unchecked(i)
            .copy_into_slice(&mut key_buf);
        msg.extend_from_slice(&key_buf);
    }
    msg
}

#[contract]
pub struct RoyaltySplitter;

#[contractimpl]
impl RoyaltySplitter {
    /// Initializes the splitter with its token, payees, splits, and the
    /// ed25519 keys that authorize reconfiguration.
    ///
    /// * `token` — the Soroban token (e.g. a standard issued-asset contract)
    ///   that subscription payments are received in.
    /// * `payees` — N stakeholder addresses, in order. The final payee
    ///   receives any fractional dust.
    /// * `splits` — basis points per payee (1 bp = 0.01%); must sum to 10,000.
    /// * `payee_keys` — ed25519 public keys aligned with `payees`, used to
    ///   verify multi-sig reconfiguration approvals.
    pub fn initialize(
        e: Env,
        token: Address,
        payees: Vec<Address>,
        splits: Vec<i128>,
        payee_keys: Vec<BytesN<32>>,
    ) -> Result<(), RoyaltyError> {
        if e.storage()
            .instance()
            .get::<DataKey, Address>(&DataKey::Token)
            .is_some()
        {
            return Err(RoyaltyError::AlreadyInitialized);
        }
        validate_config(&payees, &splits, &payee_keys)?;
        e.storage().instance().set(&DataKey::Token, &token);
        e.storage().instance().set(&DataKey::Payees, &payees);
        e.storage().instance().set(&DataKey::Splits, &splits);
        e.storage().instance().set(&DataKey::PayeeKeys, &payee_keys);
        Ok(())
    }

    /// Proposes a new configuration. The proposer must be a current
    /// stakeholder, and *every* current stakeholder must sign the proposal
    /// (via [`RoyaltySplitter::approve_reconfig`]) before it is committed.
    pub fn propose_reconfig(
        e: Env,
        proposer: Address,
        payees: Vec<Address>,
        splits: Vec<i128>,
        payee_keys: Vec<BytesN<32>>,
    ) -> Result<(), RoyaltyError> {
        let current_payees = get_payees(&e);
        if current_payees.is_empty() {
            return Err(RoyaltyError::NotInitialized);
        }
        // Only stakeholders may put a proposal on the table.
        proposer.require_auth();
        if find_payee(&current_payees, &proposer).is_none() {
            return Err(RoyaltyError::NotAStakeholder);
        }
        if e.storage()
            .instance()
            .get::<DataKey, ProposedConfig>(&DataKey::PendingConfig)
            .is_some()
        {
            return Err(RoyaltyError::PendingConfigExists);
        }
        let proposal = ProposedConfig {
            payees,
            splits,
            payee_keys,
        };
        validate_config(&proposal.payees, &proposal.splits, &proposal.payee_keys)?;
        e.storage()
            .instance()
            .set(&DataKey::PendingConfig, &proposal);
        e.storage()
            .instance()
            .set(&DataKey::Approvals, &Vec::<Address>::new(&e));
        Ok(())
    }

    /// Cancels a pending configuration. The caller must be a current
    /// stakeholder; this prevents an unapprovable proposal from blocking
    /// future reconfigurations forever.
    pub fn cancel_reconfig(e: Env, caller: Address) -> Result<(), RoyaltyError> {
        let current_payees = get_payees(&e);
        if current_payees.is_empty() {
            return Err(RoyaltyError::NotInitialized);
        }
        caller.require_auth();
        if find_payee(&current_payees, &caller).is_none() {
            return Err(RoyaltyError::NotAStakeholder);
        }
        e.storage()
            .instance()
            .get::<DataKey, ProposedConfig>(&DataKey::PendingConfig)
            .ok_or(RoyaltyError::NoPendingConfig)?;
        e.storage().instance().remove(&DataKey::PendingConfig);
        e.storage().instance().remove(&DataKey::Approvals);
        Ok(())
    }

    /// Records an ed25519-signed approval of the pending configuration from
    /// one stakeholder. Once *all* current stakeholders have approved, the
    /// new configuration is committed atomically and the proposal is cleared.
    pub fn approve_reconfig(
        e: Env,
        payee: Address,
        signature: BytesN<64>,
    ) -> Result<(), RoyaltyError> {
        let current_payees = get_payees(&e);
        if current_payees.is_empty() {
            return Err(RoyaltyError::NotInitialized);
        }
        let proposal: ProposedConfig = e
            .storage()
            .instance()
            .get(&DataKey::PendingConfig)
            .ok_or(RoyaltyError::NoPendingConfig)?;

        // The approver must be a current stakeholder (payee).
        let idx = find_payee(&current_payees, &payee).ok_or(RoyaltyError::NotAStakeholder)?;

        let mut approvals: Vec<Address> = e
            .storage()
            .instance()
            .get(&DataKey::Approvals)
            .unwrap_or_else(|| Vec::new(&e));
        if find_payee(&approvals, &payee).is_some() {
            return Err(RoyaltyError::AlreadyApproved);
        }

        // Verify the signature over the domain-separated proposal message
        // using the stakeholder's registered ed25519 public key.
        let pk = get_payee_keys(&e)
            .get(idx)
            .ok_or(RoyaltyError::NotAStakeholder)?;
        let msg = approval_message(&e, &proposal);
        e.crypto().ed25519_verify(&pk, &msg, &signature);

        approvals.push_back(payee);
        e.storage().instance().set(&DataKey::Approvals, &approvals);

        // Multi-sig gate: all current stakeholders must have approved.
        if approvals.len() == current_payees.len() {
            e.storage()
                .instance()
                .set(&DataKey::Payees, &proposal.payees);
            e.storage()
                .instance()
                .set(&DataKey::Splits, &proposal.splits);
            e.storage()
                .instance()
                .set(&DataKey::PayeeKeys, &proposal.payee_keys);
            e.storage().instance().remove(&DataKey::PendingConfig);
            e.storage().instance().remove(&DataKey::Approvals);
        }
        Ok(())
    }

    /// Processes an incoming subscription payment by pulling `amount` of the
    /// configured token from `payer` and routing floor-truncated percentage
    /// shares to each payee, with all fractional dust assigned to the final
    /// payee in the list.
    ///
    /// The payer must authorize the token transfer (directly, or via an
    /// approved subscription-operator contract). On success the contract
    /// holds zero balance: every last stroop of `amount` has been routed.
    pub fn process_payment(e: Env, payer: Address, amount: i128) -> Result<(), RoyaltyError> {
        if amount <= 0 {
            return Err(RoyaltyError::InvalidAmount);
        }
        let token: Address = e
            .storage()
            .instance()
            .get(&DataKey::Token)
            .ok_or(RoyaltyError::NotInitialized)?;
        let payees = get_payees(&e);
        if payees.is_empty() {
            return Err(RoyaltyError::NotInitialized);
        }

        // Pull the full amount into the contract first, so a
        // mid-distribution failure can never leave payees short of the
        // configured shares.
        let client = soroban_sdk::token::Client::new(&e, &token);
        client.transfer(&payer, &e.current_contract_address(), &amount);

        let payouts = compute_payouts(&e, amount)?;

        // Sequential transfers to every payee; the final one carries the dust.
        let mut distributed: i128 = 0;
        for i in 0..payees.len() {
            let payee = payees.get(i).ok_or(RoyaltyError::InvalidPayout)?;
            let payout = payouts.get(i).unwrap_or(0);
            if payout < 0 {
                return Err(RoyaltyError::InvalidPayout);
            }
            client.transfer(&e.current_contract_address(), &payee, &payout);
            distributed += payout;
        }

        if distributed != amount {
            // Unreachable (calculate_payouts preserves the total), but fail
            // loudly rather than strand dust silently.
            return Err(RoyaltyError::AmountMismatch);
        }
        Ok(())
    }

    /// Returns the current payees, splits (bps), and registered ed25519 keys.
    pub fn get_config(e: Env) -> (Vec<Address>, Vec<i128>, Vec<BytesN<32>>) {
        (get_payees(&e), get_splits(&e), get_payee_keys(&e))
    }

    /// Returns the token the splitter distributes.
    pub fn get_token(e: Env) -> Result<Address, RoyaltyError> {
        e.storage()
            .instance()
            .get(&DataKey::Token)
            .ok_or(RoyaltyError::NotInitialized)
    }

    /// Returns the pending configuration proposal, if any.
    pub fn get_pending_config(e: Env) -> Option<ProposedConfig> {
        e.storage().instance().get(&DataKey::PendingConfig)
    }

    /// Returns the stakeholders that have approved the pending configuration.
    pub fn get_approvals(e: Env) -> Vec<Address> {
        e.storage()
            .instance()
            .get(&DataKey::Approvals)
            .unwrap_or_else(|| Vec::new(&e))
    }

    /// Simulates the dust-free distribution for `amount` without moving
    /// funds. Useful for off-chain validation and UI previews.
    pub fn simulate_payouts(e: Env, amount: i128) -> Result<Vec<i128>, RoyaltyError> {
        compute_payouts(&e, amount)
    }
}
