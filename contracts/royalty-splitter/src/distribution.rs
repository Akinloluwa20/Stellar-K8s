//! Split configuration and high-precision distribution math.
//!
//! All arithmetic lives here so the contract entrypoints in [`lib.rs`] stay
//! thin and the money math can be reviewed (and unit-tested) in isolation.
//!
//! # Precision model
//!
//! Percentage splits are expressed as integer *shares* out of
//! [`TOTAL_SHARES`] = `1_000_000`. This gives six decimal places of precision,
//! which is enough to express a split such as `33.333%` (`333_330`) exactly
//! without floating point. Integer shares are used deliberately: floating point
//! is non-deterministic across VMs and must never be used on-chain.
//!
//! # Dust handling
//!
//! Integer division truncates, so `floor(amount * share_i / TOTAL)` summed over
//! every payee can be strictly less than `amount`. Rather than leaving the
//! remainder stranded in the contract, [`compute_allocation`] pays the first
//! `N - 1` payees their truncated share and assigns **the entire remaining
//! balance to the final payee**. Because the validated shares sum to exactly
//! [`TOTAL_SHARES`], that remainder is always non-negative and never exceeds
//! the final payee's entitlement by more than one minor unit.

use soroban_sdk::{contracterror, contracttype, Address, Env, Vec};

/// Denominator for all share values (parts per million = 6 decimal places).
pub const TOTAL_SHARES: u32 = 1_000_000;

/// Maximum number of payees. Bounds the O(n^2) duplicate scan and the number
/// of token transfers performed in a single invocation.
pub const MAX_PAYEES: u32 = 20;

/// A single recipient and the fraction of every payment it is owed.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Payee {
    /// Recipient of the routed funds.
    pub address: Address,
    /// Portion of every payment, denominated in [`TOTAL_SHARES`].
    pub shares: u32,
}

/// The active (or proposed) set of payees. Shares always sum to [`TOTAL_SHARES`].
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SplitConfig {
    /// Ordered payees. The order is significant: the last entry absorbs the
    /// rounding remainder, so it is the split's "dust sink".
    pub payees: Vec<Payee>,
}

/// A resolved payout produced by [`compute_allocation`].
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Allocation {
    /// Recipient of this slice.
    pub address: Address,
    /// Exact amount owed, in the payment token's minor units.
    pub amount: i128,
}

/// Errors surfaced to callers. Values are stable and should not be renumbered.
#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
#[repr(u32)]
pub enum SplitError {
    /// `initialize` was already called.
    AlreadyInitialized = 1,
    /// A split-dependent method ran before `initialize`.
    NotInitialized = 2,
    /// A configuration supplied zero payees.
    EmptyPayees = 3,
    /// More than [`MAX_PAYEES`] payees were supplied.
    TooManyPayees = 4,
    /// The same address appears more than once.
    DuplicatePayee = 5,
    /// A payee was assigned a zero share.
    ZeroShare = 6,
    /// Shares do not sum to exactly [`TOTAL_SHARES`].
    SharesDoNotSumToTotal = 7,
    /// Payment amount was zero or negative.
    InvalidAmount = 8,
    /// A multiplication or addition overflowed `i128`.
    MathOverflow = 9,
    /// Not every current payee authorized the reconfiguration.
    MissingApproval = 10,
}

/// Validate a payee set before it can become active.
///
/// A valid configuration is non-empty, bounded by [`MAX_PAYEES`], free of
/// duplicates and zero shares, and its shares sum to exactly [`TOTAL_SHARES`]
/// (full conservation of the payment).
pub fn validate(payees: &Vec<Payee>) -> Result<(), SplitError> {
    let len = payees.len();
    if len == 0 {
        return Err(SplitError::EmptyPayees);
    }
    if len > MAX_PAYEES {
        return Err(SplitError::TooManyPayees);
    }

    let mut total: u32 = 0;
    let mut i: u32 = 0;
    while i < len {
        let payee = payees.get_unchecked(i);
        if payee.shares == 0 {
            return Err(SplitError::ZeroShare);
        }

        // O(n^2) duplicate scan; `MAX_PAYEES` keeps this bounded.
        let mut j: u32 = 0;
        while j < i {
            if payees.get_unchecked(j).address == payee.address {
                return Err(SplitError::DuplicatePayee);
            }
            j += 1;
        }

        total = total
            .checked_add(payee.shares)
            .ok_or(SplitError::MathOverflow)?;
        i += 1;
    }

    if total != TOTAL_SHARES {
        return Err(SplitError::SharesDoNotSumToTotal);
    }
    Ok(())
}

/// Resolve `amount` into concrete payouts for every payee, without moving funds.
///
/// The first `N - 1` payees receive `floor(amount * shares / TOTAL_SHARES)` and
/// the final payee receives whatever is left. Given a validated configuration
/// this guarantees:
///
/// * every allocation is non-negative,
/// * the allocations sum to exactly `amount`, and
/// * the contract retains zero dust after settlement.
pub fn compute_allocation(
    env: &Env,
    amount: i128,
    payees: &Vec<Payee>,
) -> Result<Vec<Allocation>, SplitError> {
    let len = payees.len();
    if len == 0 {
        return Err(SplitError::EmptyPayees);
    }
    if amount <= 0 {
        return Err(SplitError::InvalidAmount);
    }

    let scale = TOTAL_SHARES as i128;
    let mut allocations = Vec::new(env);
    let mut distributed: i128 = 0;

    let mut i: u32 = 0;
    while i < len - 1 {
        let payee = payees.get_unchecked(i);
        let cut = amount
            .checked_mul(payee.shares as i128)
            .ok_or(SplitError::MathOverflow)?
            / scale;
        distributed = distributed.checked_add(cut).ok_or(SplitError::MathOverflow)?;
        allocations.push_back(Allocation {
            address: payee.address,
            amount: cut,
        });
        i += 1;
    }

    // The final payee absorbs the rounding remainder, so nothing is stranded.
    let last = payees.get_unchecked(len - 1);
    let remainder = amount
        .checked_sub(distributed)
        .ok_or(SplitError::MathOverflow)?;
    allocations.push_back(Allocation {
        address: last.address,
        amount: remainder,
    });

    Ok(allocations)
}

/// Return `true` when `approvals` names every payee in `payees`.
///
/// Lengths must match, which (combined with the membership check) forbids both
/// missing approvals and unrelated addresses padding the list.
pub fn approvals_cover(payees: &Vec<Payee>, approvals: &Vec<Address>) -> bool {
    if payees.len() != approvals.len() {
        return false;
    }

    let mut i: u32 = 0;
    while i < payees.len() {
        let payee = payees.get_unchecked(i);
        if !contains_address(approvals, &payee.address) {
            return false;
        }
        i += 1;
    }
    true
}

/// Linear membership test over a bounded address list.
fn contains_address(addresses: &Vec<Address>, needle: &Address) -> bool {
    let mut i: u32 = 0;
    while i < addresses.len() {
        if addresses.get_unchecked(i) == *needle {
            return true;
        }
        i += 1;
    }
    false
}

#[cfg(test)]
mod test {
    use super::*;
    use soroban_sdk::testutils::Address as _;

    fn payee(env: &Env, shares: u32) -> Payee {
        Payee {
            address: Address::generate(env),
            shares,
        }
    }

    #[test]
    fn validate_accepts_a_well_formed_split() {
        let env = Env::default();
        let mut payees = Vec::new(&env);
        payees.push_back(payee(&env, 333_330));
        payees.push_back(payee(&env, 333_330));
        payees.push_back(payee(&env, 333_340));
        assert_eq!(validate(&payees), Ok(()));
    }

    #[test]
    fn validate_rejects_bad_splits() {
        let env = Env::default();

        let empty: Vec<Payee> = Vec::new(&env);
        assert_eq!(validate(&empty), Err(SplitError::EmptyPayees));

        let mut undersubscribed = Vec::new(&env);
        undersubscribed.push_back(payee(&env, 400_000));
        undersubscribed.push_back(payee(&env, 500_000));
        assert_eq!(
            validate(&undersubscribed),
            Err(SplitError::SharesDoNotSumToTotal)
        );

        let mut zero = Vec::new(&env);
        zero.push_back(payee(&env, 0));
        zero.push_back(payee(&env, TOTAL_SHARES));
        assert_eq!(validate(&zero), Err(SplitError::ZeroShare));

        let dup = Address::generate(&env);
        let mut duplicated = Vec::new(&env);
        duplicated.push_back(Payee {
            address: dup.clone(),
            shares: 500_000,
        });
        duplicated.push_back(Payee {
            address: dup,
            shares: 500_000,
        });
        assert_eq!(validate(&duplicated), Err(SplitError::DuplicatePayee));
    }

    #[test]
    fn remainder_is_paid_to_the_last_payee() {
        let env = Env::default();
        let mut payees = Vec::new(&env);
        payees.push_back(payee(&env, 333_330));
        payees.push_back(payee(&env, 333_330));
        payees.push_back(payee(&env, 333_340));

        let allocations = compute_allocation(&env, 10_000, &payees).unwrap();
        assert_eq!(allocations.get_unchecked(0).amount, 3_333);
        assert_eq!(allocations.get_unchecked(1).amount, 3_333);
        // 10_000 - 3_333 - 3_333 = 3_334 (its own share plus the 1 unit of dust).
        assert_eq!(allocations.get_unchecked(2).amount, 3_334);
    }

    #[test]
    fn allocations_always_conserve_the_amount() {
        let env = Env::default();
        // A deliberately ugly split to maximise rounding.
        let mut payees = Vec::new(&env);
        let shares = [333_333u32, 111_111, 222_222, 333_334];
        let mut sum = 0u32;
        for s in shares {
            payees.push_back(payee(&env, s));
            sum += s;
        }
        assert_eq!(sum, TOTAL_SHARES);

        for amount in [1i128, 7, 99, 1_000_001, i128::from(u32::MAX)] {
            let allocations = compute_allocation(&env, amount, &payees).unwrap();
            let mut total = 0i128;
            for a in allocations.iter() {
                assert!(a.amount >= 0, "allocation must never be negative");
                total += a.amount;
            }
            assert_eq!(total, amount, "distribution must be dust-free");
        }
    }

    #[test]
    fn approvals_must_cover_every_payee() {
        let env = Env::default();
        let a = Address::generate(&env);
        let b = Address::generate(&env);

        let mut payees = Vec::new(&env);
        payees.push_back(Payee {
            address: a.clone(),
            shares: 500_000,
        });
        payees.push_back(Payee {
            address: b.clone(),
            shares: 500_000,
        });

        let mut partial = Vec::new(&env);
        partial.push_back(a.clone());
        assert!(!approvals_cover(&payees, &partial));

        let mut full = Vec::new(&env);
        full.push_back(a);
        full.push_back(b);
        assert!(approvals_cover(&payees, &full));
    }
}
