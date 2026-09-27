//! Deterministic, dust-free split arithmetic.
//!
//! All weights are integers so the distribution never touches floating point
//! and is byte-for-byte reproducible across hosts. Each non-final payee
//! receives `floor(amount * weight / SCALE)`; the final payee receives whatever
//! is left over.
//!
//! Flooring every leading share can only *under*-allocate, so the remainder
//! handed to the last payee is always in the range `[0, payee_count)` and
//! `sum(shares) == amount` holds exactly. This is what prevents the classic
//! `33.333% + 33.333% + 33.334%` split from stranding one unit of dust.

use soroban_sdk::{Env, Vec};

use crate::{Error, Payee, SCALE};

/// Computes the exact share (in token base units) owed to each payee.
///
/// The returned vector is index-aligned with `splits` and always sums to
/// `amount` for any `amount > 0`, with the rounding remainder assigned to the
/// final entry.
pub fn compute_shares(env: &Env, splits: &Vec<Payee>, amount: i128) -> Result<Vec<i128>, Error> {
    if amount <= 0 {
        return Err(Error::InvalidAmount);
    }
    if splits.is_empty() {
        return Err(Error::EmptySplits);
    }

    let count = splits.len();
    let mut shares: Vec<i128> = Vec::new(env);
    let mut allocated: i128 = 0;
    let mut i: u32 = 0;

    while i < count {
        let payee = splits.get(i).ok_or(Error::Overflow)?;
        let share = if i + 1 == count {
            // The final payee absorbs the rounding remainder so that
            // `sum(shares) == amount` exactly, leaving no dust behind.
            amount.checked_sub(allocated).ok_or(Error::Overflow)?
        } else {
            let share = share_floor(amount, payee.share_ppm)?;
            allocated = allocated.checked_add(share).ok_or(Error::Overflow)?;
            share
        };
        shares.push_back(share);
        i += 1;
    }

    Ok(shares)
}

/// `floor(amount * weight / SCALE)` computed without overflowing `i128`.
///
/// Splitting `amount` into a quotient and remainder around `SCALE` keeps every
/// intermediate product below `i128::MAX` even when `amount` is close to the
/// maximum token amount, so the helper is safe for arbitrarily large balances.
pub fn share_floor(amount: i128, weight: u32) -> Result<i128, Error> {
    let scale = SCALE as i128;
    let weight = weight as i128;
    let whole = amount / scale;
    let rem = amount % scale;
    let from_whole = whole.checked_mul(weight).ok_or(Error::Overflow)?;
    let from_rem = rem.checked_mul(weight).ok_or(Error::Overflow)? / scale;
    from_whole.checked_add(from_rem).ok_or(Error::Overflow)
}

/// Validates a complete split table.
///
/// A valid table is non-empty, has strictly positive weights, no duplicate
/// payee addresses, and weights that sum to exactly [`SCALE`].
pub fn validate_splits(splits: &Vec<Payee>) -> Result<(), Error> {
    if splits.is_empty() {
        return Err(Error::EmptySplits);
    }

    let count = splits.len();
    let mut total: u64 = 0;
    let mut i: u32 = 0;

    while i < count {
        let payee = splits.get(i).ok_or(Error::Overflow)?;
        if payee.share_ppm == 0 {
            return Err(Error::ZeroShare);
        }
        total += payee.share_ppm as u64;

        let mut j: u32 = 0;
        while j < i {
            if splits.get(j).ok_or(Error::Overflow)?.address == payee.address {
                return Err(Error::DuplicatePayee);
            }
            j += 1;
        }

        i += 1;
    }

    if total != SCALE as u64 {
        return Err(Error::SplitTotalMismatch);
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use soroban_sdk::{testutils::Address as _, vec, Address, Env};

    fn build(env: &Env, weights: &[u32]) -> Vec<Payee> {
        let mut splits = Vec::new(env);
        for weight in weights {
            splits.push_back(Payee {
                address: Address::generate(env),
                share_ppm: *weight,
            });
        }
        splits
    }

    fn total(shares: &Vec<i128>) -> i128 {
        let mut sum = 0i128;
        for share in shares.iter() {
            sum += share;
        }
        sum
    }

    #[test]
    fn example_three_way_split_is_dust_free() {
        // The acceptance scenario: 33.333 / 33.333 / 33.334 of 10,000 units.
        let env = Env::default();
        let splits = build(&env, &[333_330, 333_330, 333_340]);
        validate_splits(&splits).unwrap();

        let shares = compute_shares(&env, &splits, 10_000).unwrap();
        assert_eq!(shares.get(0), Some(3_333));
        assert_eq!(shares.get(1), Some(3_333));
        assert_eq!(shares.get(2), Some(3_334));
        assert_eq!(total(&shares), 10_000);
    }

    #[test]
    fn fractional_remainder_goes_to_the_last_payee() {
        let env = Env::default();
        let splits = build(&env, &[500_000, 500_000]);

        let shares = compute_shares(&env, &splits, 999).unwrap();
        assert_eq!(shares.get(0), Some(499));
        assert_eq!(shares.get(1), Some(500));
        assert_eq!(total(&shares), 999);
    }

    #[test]
    fn distribution_is_exact_across_many_amounts_and_tables() {
        let amounts: [i128; 9] = [1, 2, 3, 7, 99, 100, 10_000, 123_456_789, 100_000_000_000];
        let tables: [&[u32]; 4] = [
            &[333_330, 333_330, 333_340],
            &[500_000, 500_000],
            &[1, 1, 1, 999_997],
            &[250_000, 250_000, 250_000, 250_000],
        ];

        for table in tables {
            for amount in amounts {
                let env = Env::default();
                let splits = build(&env, table);
                let shares = compute_shares(&env, &splits, amount).unwrap();
                assert_eq!(
                    total(&shares),
                    amount,
                    "weights {table:?} lost or created units at amount {amount}"
                );
            }
        }
    }

    #[test]
    fn leading_shares_are_floored() {
        let env = Env::default();
        let splits = build(&env, &[333_330, 333_330, 333_340]);
        let shares = compute_shares(&env, &splits, 10).unwrap();
        // 10 * 0.33333 = 3.3333 -> floor 3 for each leader, last takes 4.
        assert_eq!(shares.get(0), Some(3));
        assert_eq!(shares.get(1), Some(3));
        assert_eq!(shares.get(2), Some(4));
    }

    #[test]
    fn rejects_empty_table() {
        let env = Env::default();
        assert_eq!(validate_splits(&Vec::new(&env)), Err(Error::EmptySplits));
    }

    #[test]
    fn rejects_zero_weight() {
        let env = Env::default();
        let splits = build(&env, &[0, 1_000_000]);
        assert_eq!(validate_splits(&splits), Err(Error::ZeroShare));
    }

    #[test]
    fn rejects_duplicate_payee() {
        let env = Env::default();
        let address = Address::generate(&env);
        let splits = vec![
            &env,
            Payee {
                address: address.clone(),
                share_ppm: 500_000,
            },
            Payee {
                address: address.clone(),
                share_ppm: 500_000,
            },
        ];
        assert_eq!(validate_splits(&splits), Err(Error::DuplicatePayee));
    }

    #[test]
    fn rejects_weights_that_do_not_sum_to_one() {
        let env = Env::default();
        let splits = build(&env, &[500_000, 400_000]);
        assert_eq!(validate_splits(&splits), Err(Error::SplitTotalMismatch));
    }

    #[test]
    fn rejects_non_positive_amount() {
        let env = Env::default();
        let splits = build(&env, &[1_000_000]);
        assert_eq!(compute_shares(&env, &splits, 0), Err(Error::InvalidAmount));
        assert_eq!(compute_shares(&env, &splits, -1), Err(Error::InvalidAmount));
    }
}
