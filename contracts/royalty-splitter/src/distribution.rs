//! Pure distribution math for the royalty splitter.
//!
//! Percentages are stored as basis points (1 bp = 0.01%), so `33.333%` is
//! represented exactly as `3_333` with no floating point anywhere in the
//! calculation path.
//!
//! Rounding rule (required by the splitter spec): each payee's share is
//! truncated (floored) to whole token units, and the fractional remainder —
//! the "dust" — is assigned explicitly to the **final payee in the list**.
//! This guarantees the sum of all payouts always equals the payment amount
//! exactly, so no dust can ever be stranded in the contract.

use alloc::vec::Vec;

/// Maximum representable percentage: 100% expressed in basis points.
pub const MAX_TOTAL_BPS: i128 = 10_000;

/// Validates that a single percentage split is in range `[0, 100%]`.
pub fn validate_bps(bps: i128) -> bool {
    (0..=MAX_TOTAL_BPS).contains(&bps)
}

/// Validates that a list of splits sums to exactly 100%.
///
/// `splits` must be non-empty and every entry must be individually valid.
pub fn validate_total(splits: &[i128]) -> bool {
    if splits.is_empty() {
        return false;
    }
    splits.iter().all(|&bps| validate_bps(bps)) && splits.iter().sum::<i128>() == MAX_TOTAL_BPS
}

/// Computes the whole-unit share for a single payee.
///
/// Returns `(share, fractional_remainder)` where the remainder is the dust
/// (in stroop-scale base units) that floor-truncation dropped. Empty or
/// non-positive inputs produce a zero share and carry no remainder.
pub fn compute_share(amount: i128, bps: i128) -> (i128, i128) {
    if amount <= 0 || bps <= 0 {
        return (0, 0);
    }
    // High-precision path: widen to i128 before multiplying so the
    // product cannot overflow for realistic token amounts.
    let scaled = amount * bps;
    let share = scaled / MAX_TOTAL_BPS;
    let remainder = scaled % MAX_TOTAL_BPS;
    (share, remainder)
}

/// Calculates the exact dust-free payout for every payee, in list order.
///
/// Each payee receives the floor of their percentage of `amount`; the last
/// payee additionally receives the total unclaimed dust. The returned vector
/// is guaranteed to sum to exactly `amount` whenever
/// [`validate_total`] accepts the splits, so `process_payment` can transfer
/// the full payment without stranding fractional remainder in the contract.
///
/// # Panics
///
/// Panics if `splits` is empty; callers (the contract entry points) reject
/// that configuration before reaching this function.
pub fn calculate_payouts(amount: i128, splits: &[i128]) -> Vec<i128> {
    assert!(!splits.is_empty(), "splits must not be empty");
    let len = splits.len();
    let mut payouts = Vec::with_capacity(len);
    // Total distributed to non-final payees so far; the difference to
    // `amount` is exactly the dust accumulated by floor-truncation.
    let mut distributed: i128 = 0;
    for (i, &bps) in splits.iter().enumerate() {
        let (share, _remainder) = compute_share(amount, bps);
        let payout = if i + 1 == len {
            // Final payee absorbs all accumulated dust so the sum of the
            // payouts equals `amount` exactly.
            amount - distributed
        } else {
            distributed += share;
            share
        };
        payouts.push(payout);
    }
    payouts
}

#[cfg(test)]
mod tests {
    use super::*;

    extern crate alloc;

    #[test]
    fn three_way_thirds_is_exact_and_dust_free() {
        // 33.333 / 33.333 / 33.334 (the spec's validation case).
        let splits = [3_333_i128, 3_333, 3_334];
        assert!(validate_total(&splits));

        let payouts = calculate_payouts(10_000, &splits);
        assert_eq!(payouts, alloc::vec![3_333, 3_333, 3_334]);
        assert_eq!(payouts.iter().sum::<i128>(), 10_000);
    }

    #[test]
    fn dust_is_assigned_to_the_final_payee() {
        // 10_000 * 0.25 / 0.75 splits cleanly; use odd splits that truncate.
        let splits = [5_000_i128, 2_500, 2_500];
        let payouts = calculate_payouts(10_001, &splits);
        // 10_001 * 50% = 5000.5 → 5000; 10_001 * 25% = 2500.25 → 2500 (x2);
        // the final payee absorbs the 1-unit dust.
        assert_eq!(payouts, alloc::vec![5_000, 2_500, 2_501]);
        assert_eq!(payouts.iter().sum::<i128>(), 10_001);
    }

    #[test]
    fn tiny_amounts_never_strand_dust() {
        let splits = [3_333_i128, 3_333, 3_334];
        for amount in 1..=50_i128 {
            let payouts = calculate_payouts(amount, &splits);
            assert_eq!(
                payouts.iter().sum::<i128>(),
                amount,
                "dust stranded for amount {amount}"
            );
        }
    }

    #[test]
    fn single_payee_receives_everything() {
        let splits = [10_000_i128];
        let payouts = calculate_payouts(777, &splits);
        assert_eq!(payouts, alloc::vec![777]);
    }

    #[test]
    fn validate_total_rejects_bad_configs() {
        assert!(!validate_total(&[]));
        assert!(!validate_total(&[5_000, 4_999]));
        assert!(!validate_total(&[5_000, 5_001]));
        assert!(!validate_total(&[-1, 10_001]));
        assert!(validate_total(&[10_000]));
    }
}
