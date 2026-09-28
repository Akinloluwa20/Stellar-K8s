//! Integration tests for the royalty splitter contract.
//!
//! Verifies end-to-end payment routing against the SDK's Stellar Asset
//! Contract test token (the same token interface standard asset
//! deployments expose), including the spec's validation case: 10,000
//! tokens through a 33.333% / 33.333% / 33.334% split with exact,
//! dust-free distribution.

#![cfg(test)]

extern crate std;

use crate::{RoyaltySplitter, RoyaltySplitterClient};
use ed25519_dalek::Signer;
use soroban_sdk::testutils::Address as _;
use soroban_sdk::token::{Client as TokenClient, StellarAssetClient};
use soroban_sdk::{Address, Bytes, BytesN, Env, Vec};

/// Signs a Soroban `Bytes` message with an ed25519 key (the SDK's `Bytes` is
/// converted to a host slice for `ed25519_dalek::Signer::sign`).
fn sign_bytes(key: &ed25519_dalek::SigningKey, msg: &Bytes) -> [u8; 64] {
    let mut buf = std::vec![0u8; msg.len() as usize];
    msg.copy_into_slice(&mut buf);
    key.sign(&buf).to_bytes()
}

/// Builds the domain-separated approval message client-side, mirroring the
/// contract's `approval_message`: domain tag + contract strkey + (payee,
/// split, key) tuples.
fn approval_message_for(
    e: &Env,
    contract_addr: &Address,
    payees: &Vec<Address>,
    splits: &Vec<i128>,
    keys: &Vec<BytesN<32>>,
) -> Bytes {
    let mut msg = Bytes::new(e);
    msg.extend_from_slice(b"royalty-splitter:v1:reconfig");
    let contract_id = contract_addr.to_string();
    let mut id_buf = std::vec![0u8; contract_id.len() as usize];
    contract_id.copy_into_slice(&mut id_buf);
    msg.extend_from_slice(&id_buf);
    for i in 0..payees.len() {
        let payee_str = payees.get_unchecked(i).to_string();
        let mut payee_buf = std::vec![0u8; payee_str.len() as usize];
        payee_str.copy_into_slice(&mut payee_buf);
        msg.extend_from_slice(&payee_buf);
        msg.extend_from_slice(&splits.get_unchecked(i).to_be_bytes());
        let mut key_buf = [0u8; 32];
        keys.get_unchecked(i).copy_into_slice(&mut key_buf);
        msg.extend_from_slice(&key_buf);
    }
    msg
}

/// Test fixture: a SAC token, funded payer, splitter contract, and one
/// ed25519 signing key per payee.
struct Setup {
    env: Env,
    token_id: Address,
    splitter_id: Address,
    payer: Address,
    payees: Vec<Address>,
    keys: std::vec::Vec<ed25519_dalek::SigningKey>,
}

impl Setup {
    /// Deploys the splitter with `splits` (bps) over `splits.len()` payees.
    fn new(splits: &[i128]) -> Setup {
        let env = Env::default();
        // Non-root auth is required: the splitter contract calls the token on
        // behalf of the payer and the payees (nested invocations).
        env.mock_all_auths_allowing_non_root_auth();

        let payer = Address::generate(&env);
        let payees: std::vec::Vec<Address> = (0..splits.len() as u32)
            .map(|_| Address::generate(&env))
            .collect();
        let payees: Vec<Address> = Vec::from_slice(&env, &payees);

        // One deterministic ed25519 keypair per payee.
        let keys: std::vec::Vec<ed25519_dalek::SigningKey> = (0..splits.len() as u32)
            .map(|i| {
                let mut seed = [0u8; 32];
                seed[..8].copy_from_slice(&(i as u64 + 1).to_le_bytes());
                ed25519_dalek::SigningKey::from_bytes(&seed)
            })
            .collect();
        let payee_keys: std::vec::Vec<BytesN<32>> = keys
            .iter()
            .map(|k| BytesN::<32>::from_array(&env, &k.verifying_key().to_bytes()))
            .collect();
        let payee_keys: Vec<BytesN<32>> = Vec::from_slice(&env, &payee_keys);

        // Standard issued-asset token contract (SAC) — the same interface a
        // real asset exposes on testnet/mainnet.
        let sac = env.register_stellar_asset_contract_v2(Address::generate(&env));
        let token_id = sac.address();
        // Generous balance: the amount-sweep test routes ~0.5M cumulatively.
        StellarAssetClient::new(&env, &token_id).mint(&payer, &10_000_000_i128);

        let splitter_id = env.register_contract(None, RoyaltySplitter);
        let splitter = RoyaltySplitterClient::new(&env, &splitter_id);
        let splits_vec: Vec<i128> = Vec::from_slice(&env, splits);
        splitter.initialize(&token_id, &payees, &splits_vec, &payee_keys);

        Setup {
            env,
            token_id,
            splitter_id,
            payer,
            payees,
            keys,
        }
    }

    fn token(&self) -> TokenClient<'_> {
        TokenClient::new(&self.env, &self.token_id)
    }

    fn splitter(&self) -> RoyaltySplitterClient<'_> {
        RoyaltySplitterClient::new(&self.env, &self.splitter_id)
    }

    fn balance(&self, addr: &Address) -> i128 {
        self.token().balance(addr)
    }

    fn payee_balances(&self) -> std::vec::Vec<i128> {
        self.payees.iter().map(|p| self.balance(&p)).collect()
    }
}

/// The spec's validation case: 10,000 tokens through a
/// 33.333% / 33.333% / 33.334% split must distribute exactly, dust-free.
#[test]
fn three_way_thirds_split_routes_exact_amounts() {
    let s = Setup::new(&[3_333, 3_333, 3_334]);
    let splitter = s.splitter();

    let payer_before = s.balance(&s.payer);
    let before = s.payee_balances();

    splitter.process_payment(&s.payer, &10_000);

    let after = s.payee_balances();
    assert_eq!(after[0] - before[0], 3_333);
    assert_eq!(after[1] - before[1], 3_333);
    assert_eq!(after[2] - before[2], 3_334);

    // The payer paid exactly the amount; no dust was left in the contract.
    assert_eq!(payer_before - s.balance(&s.payer), 10_000);
    assert_eq!(s.balance(&s.splitter_id), 0, "no dust stranded");
}

/// Uneven payment amounts must not strand dust; the final payee absorbs it.
#[test]
fn dust_goes_to_final_payee_on_uneven_amounts() {
    let s = Setup::new(&[3_333, 3_333, 3_334]);
    let splitter = s.splitter();

    // 10_001 * 33.33% floors to 3_333 for the first two payees, so the final
    // payee must receive 10_001 - 6_666 = 3_335.
    let before = s.payee_balances();
    splitter.process_payment(&s.payer, &10_001);
    let after = s.payee_balances();

    assert_eq!(after[0] - before[0], 3_333);
    assert_eq!(after[1] - before[1], 3_333);
    assert_eq!(after[2] - before[2], 3_335, "final payee absorbs the dust");
    assert_eq!(s.balance(&s.splitter_id), 0, "no dust stranded");
}

/// Every amount in 1..=1_000 must distribute exactly with no dust stranded.
#[test]
fn distribution_is_dust_free_across_amount_sweep() {
    let s = Setup::new(&[3_333, 3_333, 3_334]);
    let splitter = s.splitter();

    for amount in 1..=1_000i128 {
        // Each invocation consumes CPU budget; reset it periodically so the
        // sweep only measures distribution math, not cumulative budget.
        s.env.budget().reset_unlimited();
        let before = s.payee_balances();
        splitter.process_payment(&s.payer, &amount);
        let after = s.payee_balances();
        let distributed: i128 = after.iter().zip(before.iter()).map(|(a, b)| a - b).sum();
        assert_eq!(distributed, amount, "dust stranded for amount {amount}");
        assert_eq!(
            s.balance(&s.splitter_id),
            0,
            "contract held dust at {amount}"
        );
    }
}

/// Individually truncating splits over four payees still distribute exactly.
#[test]
fn odd_percentages_and_many_payees_stay_dust_free() {
    let s = Setup::new(&[1_250, 3_750, 1_999, 3_001]);
    let splitter = s.splitter();

    // 7 units: every percentage share floors to 0 except via dust assignment.
    let before = s.payee_balances();
    splitter.process_payment(&s.payer, &7);
    let after = s.payee_balances();

    // 12.5% of 7 = 0.875 → 0; 37.5% of 7 = 2.625 → 2; 19.999% of 7 → 1;
    // final payee (30.001%) gets 7 - 0 - 2 - 1 = 4.
    assert_eq!(after[0] - before[0], 0);
    assert_eq!(after[1] - before[1], 2);
    assert_eq!(after[2] - before[2], 1);
    assert_eq!(after[3] - before[3], 4);
    assert_eq!(s.balance(&s.splitter_id), 0, "no dust stranded");
}

#[test]
fn config_is_readable_after_init() {
    let s = Setup::new(&[3_333, 3_333, 3_334]);
    let (payees, splits, keys) = s.splitter().get_config();
    assert_eq!(payees.len(), 3);
    assert_eq!(splits.get(0), Some(3_333));
    assert_eq!(splits.get(1), Some(3_333));
    assert_eq!(splits.get(2), Some(3_334));
    assert_eq!(keys.len(), 3);
}

#[test]
fn simulate_matches_expected_distribution() {
    let s = Setup::new(&[3_333, 3_333, 3_334]);
    let payouts = s.splitter().simulate_payouts(&10_000);
    assert_eq!(payouts.get(0), Some(3_333));
    assert_eq!(payouts.get(1), Some(3_333));
    assert_eq!(payouts.get(2), Some(3_334));
}

#[test]
fn rejects_split_sum_not_equal_to_100_percent() {
    // Fresh env/contract so initialization itself is what's being validated.
    let env = Env::default();
    env.mock_all_auths_allowing_non_root_auth();

    let sac = env.register_stellar_asset_contract_v2(Address::generate(&env));
    let token_id = sac.address();
    let payees: std::vec::Vec<Address> = (0..2).map(|_| Address::generate(&env)).collect();
    let payees = Vec::from_slice(&env, &payees);
    let keys: std::vec::Vec<BytesN<32>> = (0..2)
        .map(|i| {
            let mut seed = [0u8; 32];
            seed[0] = i as u8 + 1;
            BytesN::<32>::from_array(&env, &seed)
        })
        .collect();
    let keys = Vec::from_slice(&env, &keys);

    let splitter_id = env.register_contract(None, RoyaltySplitter);
    let splitter = RoyaltySplitterClient::new(&env, &splitter_id);

    // 9,999 bps ≠ 100%.
    let bad_sum = Vec::from_slice(&env, &[5_000, 4_999]);
    assert_eq!(
        splitter.try_initialize(&token_id, &payees, &bad_sum, &keys),
        Err(Ok(crate::RoyaltyError::SplitSumInvalid))
    );
}

#[test]
fn rejects_zero_and_negative_payment_amounts() {
    let s = Setup::new(&[3_333, 3_333, 3_334]);
    let splitter = s.splitter();
    assert_eq!(
        splitter.try_process_payment(&s.payer, &0),
        Err(Ok(crate::RoyaltyError::InvalidAmount))
    );
    assert_eq!(
        splitter.try_process_payment(&s.payer, &-1),
        Err(Ok(crate::RoyaltyError::InvalidAmount))
    );
}

/// Multi-sig: the configuration only changes once every stakeholder has
/// approved, and the committed configuration routes dust-free payments.
#[test]
fn reconfig_requires_all_stakeholder_signatures() {
    let s = Setup::new(&[3_333, 3_333, 3_334]);
    let splitter = s.splitter();

    let new_payees: Vec<Address> = Vec::from_slice(
        &s.env,
        &[Address::generate(&s.env), Address::generate(&s.env)],
    );
    let new_keys: Vec<BytesN<32>> = Vec::from_slice(
        &s.env,
        &[
            BytesN::<32>::from_array(&s.env, &[9u8; 32]),
            BytesN::<32>::from_array(&s.env, &[8u8; 32]),
        ],
    );
    let new_splits: Vec<i128> = Vec::from_slice(&s.env, &[5_000, 5_000]);

    // Propose replacing the current payees with two 50/50 payees.
    splitter.propose_reconfig(
        &s.payees.get(0).unwrap(),
        &new_payees,
        &new_splits,
        &new_keys,
    );
    let proposal = splitter
        .get_pending_config()
        .expect("expected pending config");

    // Partial approval must not change the live configuration.
    let msg = approval_message_for(
        &s.env,
        &s.splitter_id,
        &proposal.payees,
        &proposal.splits,
        &proposal.payee_keys,
    );
    let sig = BytesN::<64>::from_array(&s.env, &sign_bytes(&s.keys[0], &msg));
    splitter.approve_reconfig(&s.payees.get(0).unwrap(), &sig);

    let (payees, splits, _) = splitter.get_config();
    assert_eq!(payees.len(), 3, "config unchanged before full approval");
    assert_eq!(splits.get(0), Some(3_333));
    assert_eq!(splitter.get_approvals().len(), 1);

    // Remaining approvals commit the new configuration atomically.
    for (payee, key) in s.payees.iter().skip(1).zip(s.keys.iter().skip(1)) {
        let sig = BytesN::<64>::from_array(&s.env, &sign_bytes(key, &msg));
        splitter.approve_reconfig(&payee, &sig);
    }

    let (payees, splits, _) = splitter.get_config();
    assert_eq!(payees.len(), 2, "config committed after full approval");
    assert_eq!(splits.get(0), Some(5_000));
    assert_eq!(splits.get(1), Some(5_000));
    assert!(splitter.get_pending_config().is_none());
    assert_eq!(splitter.get_approvals().len(), 0);

    // The new configuration routes payments dust-free: 10,000 → 5,000/5,000.
    splitter.process_payment(&s.payer, &10_000);
    assert_eq!(s.balance(&s.splitter_id), 0);
}

/// A signature from a key that is not registered to the approving payee must
/// be rejected, and the approval must not be recorded.
#[test]
fn reconfig_rejects_wrong_signer() {
    let s = Setup::new(&[3_333, 3_333, 3_334]);
    let splitter = s.splitter();

    let new_payees: Vec<Address> = Vec::from_slice(
        &s.env,
        &[Address::generate(&s.env), Address::generate(&s.env)],
    );
    let new_keys: Vec<BytesN<32>> = Vec::from_slice(
        &s.env,
        &[
            BytesN::<32>::from_array(&s.env, &[9u8; 32]),
            BytesN::<32>::from_array(&s.env, &[8u8; 32]),
        ],
    );
    let new_splits: Vec<i128> = Vec::from_slice(&s.env, &[5_000, 5_000]);

    splitter.propose_reconfig(
        &s.payees.get(0).unwrap(),
        &new_payees,
        &new_splits,
        &new_keys,
    );
    let proposal = splitter
        .get_pending_config()
        .expect("expected pending config");
    let msg = approval_message_for(
        &s.env,
        &s.splitter_id,
        &proposal.payees,
        &proposal.splits,
        &proposal.payee_keys,
    );

    // Sign with a key that is not registered to the first payee. An invalid
    // ed25519 signature traps in the host (fail-closed), surfacing as an
    // aborted invocation on the client.
    let impostor = ed25519_dalek::SigningKey::from_bytes(&[7u8; 32]);
    let bad_sig = BytesN::<64>::from_array(&s.env, &sign_bytes(&impostor, &msg));
    assert_eq!(
        splitter.try_approve_reconfig(&s.payees.get(0).unwrap(), &bad_sig),
        Err(Err(soroban_sdk::InvokeError::Abort))
    );
    assert_eq!(splitter.get_approvals().len(), 0);
}

/// The same approval cannot be recorded twice.
#[test]
fn reconfig_rejects_duplicate_approval() {
    let s = Setup::new(&[3_333, 3_333, 3_334]);
    let splitter = s.splitter();

    let new_payees: Vec<Address> = Vec::from_slice(
        &s.env,
        &[Address::generate(&s.env), Address::generate(&s.env)],
    );
    let new_keys: Vec<BytesN<32>> = Vec::from_slice(
        &s.env,
        &[
            BytesN::<32>::from_array(&s.env, &[9u8; 32]),
            BytesN::<32>::from_array(&s.env, &[8u8; 32]),
        ],
    );
    let new_splits: Vec<i128> = Vec::from_slice(&s.env, &[5_000, 5_000]);

    splitter.propose_reconfig(
        &s.payees.get(0).unwrap(),
        &new_payees,
        &new_splits,
        &new_keys,
    );
    let proposal = splitter
        .get_pending_config()
        .expect("expected pending config");
    let msg = approval_message_for(
        &s.env,
        &s.splitter_id,
        &proposal.payees,
        &proposal.splits,
        &proposal.payee_keys,
    );
    let sig = BytesN::<64>::from_array(&s.env, &sign_bytes(&s.keys[0], &msg));

    splitter.approve_reconfig(&s.payees.get(0).unwrap(), &sig);
    assert_eq!(
        splitter.try_approve_reconfig(&s.payees.get(0).unwrap(), &sig),
        Err(Ok(crate::RoyaltyError::AlreadyApproved))
    );
}

/// A stakeholder can cancel a pending proposal, clearing the way for a new one.
#[test]
fn stakeholder_can_cancel_pending_reconfig() {
    let s = Setup::new(&[3_333, 3_333, 3_334]);
    let splitter = s.splitter();

    let new_payees: Vec<Address> = Vec::from_slice(
        &s.env,
        &[Address::generate(&s.env), Address::generate(&s.env)],
    );
    let new_keys: Vec<BytesN<32>> = Vec::from_slice(
        &s.env,
        &[
            BytesN::<32>::from_array(&s.env, &[9u8; 32]),
            BytesN::<32>::from_array(&s.env, &[8u8; 32]),
        ],
    );
    let new_splits: Vec<i128> = Vec::from_slice(&s.env, &[5_000, 5_000]);

    splitter.propose_reconfig(
        &s.payees.get(0).unwrap(),
        &new_payees,
        &new_splits,
        &new_keys,
    );
    assert!(splitter.get_pending_config().is_some());

    splitter.cancel_reconfig(&s.payees.get(1).unwrap());
    assert!(splitter.get_pending_config().is_none());
    assert_eq!(splitter.get_approvals().len(), 0);
}
