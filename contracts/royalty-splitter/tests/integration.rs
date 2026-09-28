//! End-to-end tests for the royalty splitter.
//!
//! Every test drives the contract through the standard Soroban token interface
//! backed by a real **Stellar Asset Contract** (`register_stellar_asset_contract_v2`),
//! which is how value actually moves on Stellar. This verifies the splitter is
//! compatible with vanilla Soroban asset transfers rather than a bespoke mock.

use royalty_splitter::{Payee, RoyaltySplitter, RoyaltySplitterClient};
use soroban_sdk::{
    testutils::Address as _,
    token::{Client as TokenClient, StellarAssetClient},
    Address, Env, Vec,
};

/// Deploy a fresh splitter contract and return its id + generated client.
fn deploy_splitter<'a>(env: &'a Env) -> (Address, RoyaltySplitterClient<'a>) {
    let contract_id = env.register(RoyaltySplitter, ());
    let client = RoyaltySplitterClient::new(env, &contract_id);
    (contract_id, client)
}

/// Deploy a standard Stellar Asset Contract and the clients used to drive it.
fn deploy_token<'a>(env: &'a Env) -> (Address, StellarAssetClient<'a>, TokenClient<'a>) {
    let admin = Address::generate(env);
    let token_id = env.register_stellar_asset_contract_v2(admin).address();
    (
        token_id.clone(),
        StellarAssetClient::new(env, &token_id),
        TokenClient::new(env, &token_id),
    )
}

/// Build a payee list from `(address, shares)` pairs.
fn make_payees(env: &Env, entries: &[(Address, u32)]) -> Vec<Payee> {
    let mut payees = Vec::new(env);
    for (address, shares) in entries.iter() {
        payees.push_back(Payee {
            address: address.clone(),
            shares: *shares,
        });
    }
    payees
}

/// The headline scenario from the specification: a 33.333% / 33.333% / 33.334%
/// split of 10,000 tokens must settle to the exact minor unit with no dust.
#[test]
fn splits_10_000_tokens_without_dust() {
    let env = Env::default();
    env.mock_all_auths();

    let payer = Address::generate(&env);
    let creator = Address::generate(&env);
    let platform = Address::generate(&env);
    let affiliate = Address::generate(&env);

    let (token_id, asset, token) = deploy_token(&env);
    let (contract_id, splitter) = deploy_splitter(&env);

    let config = make_payees(
        &env,
        &[
            (creator.clone(), 333_330),   // 33.333%
            (platform.clone(), 333_330),  // 33.333%
            (affiliate.clone(), 333_340), // 33.334%
        ],
    );
    splitter.initialize(&config);

    let amount = 10_000i128;
    asset.mint(&payer, &amount);
    splitter.process_payment(&payer, &token_id, &amount);

    // Truncation drops 0.3 of a unit from each of the first two payees; the
    // accumulated remainder is paid to the final payee.
    assert_eq!(token.balance(&creator), 3_333);
    assert_eq!(token.balance(&platform), 3_333);
    assert_eq!(token.balance(&affiliate), 3_334);

    // Nothing is left behind, and the payer is fully debited.
    assert_eq!(token.balance(&contract_id), 0);
    assert_eq!(token.balance(&payer), 0);

    // Conservation of value: the payouts sum to exactly the payment.
    let routed = token.balance(&creator) + token.balance(&platform) + token.balance(&affiliate);
    assert_eq!(routed, amount);
}

/// `preview` must agree with what `process_payment` actually does, for an
/// awkward many-payee split and a prime-ish amount that guarantees rounding.
#[test]
fn preview_matches_settlement_and_strands_no_dust() {
    let env = Env::default();
    env.mock_all_auths();

    let payer = Address::generate(&env);
    let (token_id, asset, token) = deploy_token(&env);
    let (contract_id, splitter) = deploy_splitter(&env);

    let recipients = [
        Address::generate(&env),
        Address::generate(&env),
        Address::generate(&env),
        Address::generate(&env),
        Address::generate(&env),
        Address::generate(&env),
        Address::generate(&env),
    ];
    // 7 uneven shares summing to exactly 1_000_000.
    let shares = [
        200_000u32, 150_000, 120_000, 110_000, 140_000, 130_000, 150_000,
    ];
    assert_eq!(shares.iter().sum::<u32>(), 1_000_000);

    let mut config = Vec::new(&env);
    for (address, share) in recipients.iter().zip(shares.iter()) {
        config.push_back(Payee {
            address: address.clone(),
            shares: *share,
        });
    }
    splitter.initialize(&config);

    let amount = 1_000_003i128;
    asset.mint(&payer, &amount);

    let expected = splitter.preview(&amount);
    assert_eq!(expected.len(), 7);

    splitter.process_payment(&payer, &token_id, &amount);

    let mut settled = 0i128;
    for allocation in expected.iter() {
        assert_eq!(token.balance(&allocation.address), allocation.amount);
        settled += allocation.amount;
    }
    assert_eq!(
        settled, amount,
        "preview allocations must sum to the payment"
    );
    assert_eq!(token.balance(&contract_id), 0, "contract must hold no dust");
}

/// Reconfiguration is gated on unanimity from the current payees.
#[test]
fn reconfiguration_requires_unanimous_approval() {
    let env = Env::default();
    env.mock_all_auths();

    let a = Address::generate(&env);
    let b = Address::generate(&env);
    let c = Address::generate(&env);
    let d = Address::generate(&env);

    let (_contract_id, splitter) = deploy_splitter(&env);
    splitter.initialize(&make_payees(
        &env,
        &[(a.clone(), 500_000), (b.clone(), 500_000)],
    ));

    let new_config = make_payees(&env, &[(c.clone(), 600_000), (d.clone(), 400_000)]);

    // Only one of the two current payees approves -> rejected.
    let mut partial = Vec::new(&env);
    partial.push_back(a.clone());
    assert!(splitter.try_update_splits(&new_config, &partial).is_err());

    // An unrelated address does not count as approval.
    let mut stranger = Vec::new(&env);
    stranger.push_back(a.clone());
    stranger.push_back(Address::generate(&env));
    assert!(splitter.try_update_splits(&new_config, &stranger).is_err());

    // Unanimous approval succeeds.
    let mut unanimous = Vec::new(&env);
    unanimous.push_back(a);
    unanimous.push_back(b);
    splitter.update_splits(&new_config, &unanimous);

    let active = splitter.get_config();
    assert_eq!(active.payees.len(), 2);
    assert_eq!(active.payees.get_unchecked(0).shares, 600_000);
}

/// After a reconfiguration, future payments follow the new split.
#[test]
fn reconfiguration_redirects_future_payments() {
    let env = Env::default();
    env.mock_all_auths();

    let payer = Address::generate(&env);
    let old_a = Address::generate(&env);
    let old_b = Address::generate(&env);
    let new_a = Address::generate(&env);
    let new_b = Address::generate(&env);

    let (token_id, asset, token) = deploy_token(&env);
    let (contract_id, splitter) = deploy_splitter(&env);

    splitter.initialize(&make_payees(
        &env,
        &[(old_a.clone(), 500_000), (old_b.clone(), 500_000)],
    ));

    let mut approvals = Vec::new(&env);
    approvals.push_back(old_a.clone());
    approvals.push_back(old_b.clone());
    splitter.update_splits(
        &make_payees(&env, &[(new_a.clone(), 700_000), (new_b.clone(), 300_000)]),
        &approvals,
    );

    asset.mint(&payer, &1_000);
    splitter.process_payment(&payer, &token_id, &1_000);

    assert_eq!(token.balance(&old_a), 0);
    assert_eq!(token.balance(&old_b), 0);
    assert_eq!(token.balance(&new_a), 700);
    assert_eq!(token.balance(&new_b), 300);
    assert_eq!(token.balance(&contract_id), 0);
}

/// The contract exposes running counters for indexers and dashboards.
#[test]
fn tracks_payment_counters() {
    let env = Env::default();
    env.mock_all_auths();

    let payer = Address::generate(&env);
    let a = Address::generate(&env);
    let b = Address::generate(&env);

    let (token_id, asset, token) = deploy_token(&env);
    let (contract_id, splitter) = deploy_splitter(&env);
    splitter.initialize(&make_payees(
        &env,
        &[(a.clone(), 500_000), (b.clone(), 500_000)],
    ));

    assert_eq!(splitter.payments_processed(), 0);
    assert_eq!(splitter.total_routed(), 0);

    asset.mint(&payer, &2_500);
    splitter.process_payment(&payer, &token_id, &1_000);
    splitter.process_payment(&payer, &token_id, &1_500);

    assert_eq!(splitter.payments_processed(), 2);
    assert_eq!(splitter.total_routed(), 2_500);
    assert_eq!(token.balance(&contract_id), 0);
}

/// Invalid splits are rejected before the contract can be used.
#[test]
fn initialization_rejects_invalid_splits() {
    let env = Env::default();
    env.mock_all_auths();

    let a = Address::generate(&env);
    let b = Address::generate(&env);

    let (_contract_id, splitter) = deploy_splitter(&env);

    // Shares that do not sum to 1_000_000 are refused.
    let bad_sum = make_payees(&env, &[(a.clone(), 400_000), (b.clone(), 500_000)]);
    assert!(splitter.try_initialize(&bad_sum).is_err());

    // A zero share is refused even if the total still adds up.
    let zero_share = make_payees(&env, &[(a.clone(), 0), (b.clone(), 1_000_000)]);
    assert!(splitter.try_initialize(&zero_share).is_err());

    // Duplicate recipients are refused.
    let duplicate = make_payees(&env, &[(a.clone(), 500_000), (a.clone(), 500_000)]);
    assert!(splitter.try_initialize(&duplicate).is_err());
}

/// `initialize` is one-shot and `process_payment` cannot run before setup.
#[test]
fn lifecycle_guards_are_enforced() {
    let env = Env::default();
    env.mock_all_auths();

    let payer = Address::generate(&env);
    let a = Address::generate(&env);
    let b = Address::generate(&env);

    let (token_id, asset, _token) = deploy_token(&env);
    let (_contract_id, splitter) = deploy_splitter(&env);

    // Payment before initialization is rejected.
    asset.mint(&payer, &100);
    assert!(splitter
        .try_process_payment(&payer, &token_id, &100)
        .is_err());

    let config = make_payees(&env, &[(a, 500_000), (b, 500_000)]);
    splitter.initialize(&config);

    // Re-initialization is rejected.
    assert!(splitter.try_initialize(&config).is_err());
}
