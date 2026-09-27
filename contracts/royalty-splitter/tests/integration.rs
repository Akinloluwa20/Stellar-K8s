//! End-to-end tests for the royalty splitter against a real Stellar Asset
//! Contract (SAC), which is what standard Soroban token transfers run on.

use royalty_splitter::{Error, Payee, RoyaltySplitter, RoyaltySplitterClient};
use soroban_sdk::{testutils::Address as _, token, vec, Address, Env, Vec};

/// Registers a Stellar Asset Contract plus the splitter, returning the splitter
/// client together with the admin, token and splitter contract addresses.
fn deploy(env: &Env) -> (RoyaltySplitterClient<'_>, Address, Address, Address) {
    env.mock_all_auths();

    let admin = Address::generate(env);
    let sac = env.register_stellar_asset_contract_v2(admin.clone());
    let token_id = sac.address();

    let contract_id = env.register(RoyaltySplitter, ());
    let client = RoyaltySplitterClient::new(env, &contract_id);

    (client, admin, token_id, contract_id)
}

fn balance(env: &Env, token_id: &Address, who: &Address) -> i128 {
    token::TokenClient::new(env, token_id).balance(who)
}

/// The acceptance scenario from the issue: 10,000 units through a
/// 33.333% / 33.333% / 33.334% split must be dust-free.
#[test]
fn fractional_three_way_split_routes_every_unit() {
    let env = Env::default();
    let (client, admin, token_id, contract_id) = deploy(&env);

    let payer = Address::generate(&env);
    let alice = Address::generate(&env);
    let bob = Address::generate(&env);
    let carol = Address::generate(&env);

    let splits = vec![
        &env,
        Payee {
            address: alice.clone(),
            share_ppm: 333_330,
        },
        Payee {
            address: bob.clone(),
            share_ppm: 333_330,
        },
        Payee {
            address: carol.clone(),
            share_ppm: 333_340,
        },
    ];

    client.initialize(&admin, &token_id, &splits);

    let minted: i128 = 10_000;
    token::StellarAssetClient::new(&env, &token_id).mint(&payer, &minted);

    let shares = client.process_payment(&payer, &minted);

    assert_eq!(shares, vec![&env, 3_333, 3_333, 3_334]);
    assert_eq!(shares.iter().sum::<i128>(), minted);

    // Every unit moved through standard SAC transfers, none left behind.
    assert_eq!(balance(&env, &token_id, &payer), 0);
    assert_eq!(balance(&env, &token_id, &alice), 3_333);
    assert_eq!(balance(&env, &token_id, &bob), 3_333);
    assert_eq!(balance(&env, &token_id, &carol), 3_334);
    assert_eq!(
        balance(&env, &token_id, &contract_id),
        0,
        "splitter must not retain any dust"
    );
}

/// A two-way split with an odd amount must still allocate the exact total.
#[test]
fn odd_amount_is_fully_distributed() {
    let env = Env::default();
    let (client, admin, token_id, _contract_id) = deploy(&env);

    let payer = Address::generate(&env);
    let alice = Address::generate(&env);
    let bob = Address::generate(&env);

    let splits = vec![
        &env,
        Payee {
            address: alice.clone(),
            share_ppm: 500_000,
        },
        Payee {
            address: bob.clone(),
            share_ppm: 500_000,
        },
    ];
    client.initialize(&admin, &token_id, &splits);
    token::StellarAssetClient::new(&env, &token_id).mint(&payer, &999);

    let shares = client.process_payment(&payer, &999);

    assert_eq!(shares, vec![&env, 499, 500]);
    assert_eq!(balance(&env, &token_id, &alice), 499);
    assert_eq!(balance(&env, &token_id, &bob), 500);
}

#[test]
fn repeated_payments_never_accumulate_dust() {
    let env = Env::default();
    let (client, admin, token_id, contract_id) = deploy(&env);

    let payer = Address::generate(&env);
    let payees = [
        Address::generate(&env),
        Address::generate(&env),
        Address::generate(&env),
    ];

    let splits = vec![
        &env,
        Payee {
            address: payees[0].clone(),
            share_ppm: 333_330,
        },
        Payee {
            address: payees[1].clone(),
            share_ppm: 333_330,
        },
        Payee {
            address: payees[2].clone(),
            share_ppm: 333_340,
        },
    ];
    client.initialize(&admin, &token_id, &splits);

    let asset = token::StellarAssetClient::new(&env, &token_id);
    let per_payment: i128 = 1_000;
    let payments: i128 = 7;
    asset.mint(&payer, &(per_payment * payments));

    for _ in 0..payments {
        let shares = client.process_payment(&payer, &per_payment);
        assert_eq!(shares.iter().sum::<i128>(), per_payment);
    }

    assert_eq!(balance(&env, &token_id, &payer), 0);
    assert_eq!(balance(&env, &token_id, &contract_id), 0);
    let routed: i128 = payees
        .iter()
        .map(|payee| balance(&env, &token_id, payee))
        .sum();
    assert_eq!(routed, per_payment * payments);
}

#[test]
fn reconfiguration_requires_every_current_stakeholder() {
    let env = Env::default();
    let (client, admin, token_id, _contract_id) = deploy(&env);

    let stakeholder_a = Address::generate(&env);
    let stakeholder_b = Address::generate(&env);
    let new_payee = Address::generate(&env);
    let outsider = Address::generate(&env);
    let payer = Address::generate(&env);

    let initial = vec![
        &env,
        Payee {
            address: stakeholder_a.clone(),
            share_ppm: 500_000,
        },
        Payee {
            address: stakeholder_b.clone(),
            share_ppm: 500_000,
        },
    ];
    client.initialize(&admin, &token_id, &initial);

    let updated = vec![
        &env,
        Payee {
            address: new_payee.clone(),
            share_ppm: 1_000_000,
        },
    ];
    client.propose_splits(&stakeholder_a, &updated);

    // A non-stakeholder cannot approve.
    assert_eq!(
        client.try_approve_splits(&outsider),
        Err(Ok(Error::NotAStakeholder))
    );

    // A single approval is not enough, and duplicates are rejected.
    assert!(!client.approve_splits(&stakeholder_a));
    assert_eq!(
        client.try_approve_splits(&stakeholder_a),
        Err(Ok(Error::AlreadyApproved))
    );
    assert_eq!(client.get_splits(), initial);

    // The final approval applies the configuration.
    assert!(client.approve_splits(&stakeholder_b));
    assert_eq!(client.get_splits(), updated);
    assert_eq!(client.get_pending(), None);

    // Payments now route only to the new payee.
    token::StellarAssetClient::new(&env, &token_id).mint(&payer, &10_000);
    assert_eq!(client.process_payment(&payer, &10_000), vec![&env, 10_000]);
    assert_eq!(balance(&env, &token_id, &new_payee), 10_000);
    assert_eq!(balance(&env, &token_id, &stakeholder_a), 0);
    assert_eq!(balance(&env, &token_id, &stakeholder_b), 0);
}

#[test]
fn only_proposer_or_admin_can_cancel() {
    let env = Env::default();
    let (client, admin, token_id, _contract_id) = deploy(&env);

    let stakeholder = Address::generate(&env);
    let other = Address::generate(&env);
    let splits = vec![
        &env,
        Payee {
            address: stakeholder.clone(),
            share_ppm: 1_000_000,
        },
    ];
    client.initialize(&admin, &token_id, &splits);

    let updated = vec![
        &env,
        Payee {
            address: other.clone(),
            share_ppm: 1_000_000,
        },
    ];

    // The admin may propose even though it is not a payee.
    client.propose_splits(&admin, &updated);
    assert!(client.get_pending().is_some());

    // The stakeholder is neither the proposer nor the admin.
    assert_eq!(
        client.try_cancel_splits(&stakeholder),
        Err(Ok(Error::Unauthorized))
    );

    // The admin (also the proposer) can cancel, restoring the original table.
    client.cancel_splits(&admin);
    assert!(client.get_pending().is_none());
    assert_eq!(client.get_splits(), splits);
}

#[test]
fn non_stakeholder_cannot_propose() {
    let env = Env::default();
    let (client, admin, token_id, _contract_id) = deploy(&env);

    let stakeholder = Address::generate(&env);
    let outsider = Address::generate(&env);
    let splits = vec![
        &env,
        Payee {
            address: stakeholder.clone(),
            share_ppm: 1_000_000,
        },
    ];
    client.initialize(&admin, &token_id, &splits);

    let updated = vec![
        &env,
        Payee {
            address: outsider.clone(),
            share_ppm: 1_000_000,
        },
    ];
    assert_eq!(
        client.try_propose_splits(&outsider, &updated),
        Err(Ok(Error::Unauthorized))
    );
}

#[test]
fn proposal_must_differ_from_active_configuration() {
    let env = Env::default();
    let (client, admin, token_id, _contract_id) = deploy(&env);

    let stakeholder = Address::generate(&env);
    let splits = vec![
        &env,
        Payee {
            address: stakeholder.clone(),
            share_ppm: 1_000_000,
        },
    ];
    client.initialize(&admin, &token_id, &splits);

    assert_eq!(
        client.try_propose_splits(&admin, &splits),
        Err(Ok(Error::ConfigUnchanged))
    );
}

#[test]
fn rejects_misaligned_split_table() {
    let env = Env::default();
    let (client, admin, token_id, _contract_id) = deploy(&env);

    let alice = Address::generate(&env);
    let bob = Address::generate(&env);
    // 50% + 40% != 100%.
    let bad = vec![
        &env,
        Payee {
            address: alice.clone(),
            share_ppm: 500_000,
        },
        Payee {
            address: bob.clone(),
            share_ppm: 400_000,
        },
    ];

    assert_eq!(
        client.try_initialize(&admin, &token_id, &bad),
        Err(Ok(Error::SplitTotalMismatch))
    );
}

#[test]
fn uninitialized_contract_rejects_payments() {
    let env = Env::default();
    env.mock_all_auths();

    let payer = Address::generate(&env);
    let contract_id = env.register(RoyaltySplitter, ());
    let client = RoyaltySplitterClient::new(&env, &contract_id);

    assert_eq!(
        client.try_process_payment(&payer, &100),
        Err(Ok(Error::NotInitialized))
    );
}

#[test]
fn rejects_non_positive_payment() {
    let env = Env::default();
    let (client, admin, token_id, _contract_id) = deploy(&env);

    let payer = Address::generate(&env);
    let stakeholder = Address::generate(&env);
    let splits = Vec::from_array(
        &env,
        [Payee {
            address: stakeholder.clone(),
            share_ppm: 1_000_000,
        }],
    );
    client.initialize(&admin, &token_id, &splits);

    assert_eq!(
        client.try_process_payment(&payer, &0),
        Err(Ok(Error::InvalidAmount))
    );
    assert_eq!(
        client.try_process_payment(&payer, &-5),
        Err(Ok(Error::InvalidAmount))
    );
}
