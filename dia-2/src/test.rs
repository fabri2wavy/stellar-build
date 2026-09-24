#![cfg(test)]

use super::*;
use soroban_sdk::{
    testutils::{Address as _, MockAuth, MockAuthInvoke},
    IntoVal,
};

// Convierte nuestro codigo de error al tipo del SDK.
// Se utiliza al probar funciones que devuelven ().
fn sdk_error(error: Error) -> soroban_sdk::Error {
    soroban_sdk::Error::from_contract_error(error as u32)
}

fn sample_asset(env: &Env) -> AssetInfo {
    AssetInfo {
        name: Symbol::new(env, "SBDEMO"),
        total_supply: 1_000_000,
        price_per_unit: 100,
        payment_token: Address::generate(env),
        paused: false,
    }
}

fn setup(env: &Env) -> (Address, Address) {
    let id = env.register(RwaLaunchpad, ());
    let admin = Address::generate(env);

    env.mock_all_auths();

    let client = RwaLaunchpadClient::new(env, &id);
    client.initialize(&admin, &sample_asset(env));

    (id, admin)
}

#[test]
fn test_mint_and_transfer() {
    let env = Env::default();
    let (id, admin) = setup(&env);
    let client = RwaLaunchpadClient::new(&env, &id);
    let alice = Address::generate(&env);
    let bob = Address::generate(&env);

    client.mint(&admin, &alice, &1_000);
    assert_eq!(client.balance(&alice), 1_000);

    client.transfer(&alice, &bob, &400);

    assert_eq!(client.balance(&alice), 600);
    assert_eq!(client.balance(&bob), 400);
}

#[test]
fn test_self_transfer_preserves_balance() {
    let env = Env::default();
    let (id, admin) = setup(&env);
    let client = RwaLaunchpadClient::new(&env, &id);
    let investor = Address::generate(&env);

    client.mint(&admin, &investor, &500);
    client.transfer(&investor, &investor, &200);

    assert_eq!(client.balance(&investor), 500);
}

#[test]
fn test_insufficient_balance_changes_nothing() {
    let env = Env::default();
    let (id, admin) = setup(&env);
    let client = RwaLaunchpadClient::new(&env, &id);
    let alice = Address::generate(&env);
    let bob = Address::generate(&env);

    client.mint(&admin, &alice, &100);

    assert_eq!(
        client.try_transfer(&alice, &bob, &200),
        Err(Ok(sdk_error(Error::InsufficientBalance)))
    );

    assert_eq!(client.balance(&alice), 100);
    assert_eq!(client.balance(&bob), 0);
}

#[test]
fn test_invalid_amounts() {
    let env = Env::default();
    let (id, admin) = setup(&env);
    let client = RwaLaunchpadClient::new(&env, &id);
    let alice = Address::generate(&env);
    let bob = Address::generate(&env);

    assert_eq!(
        client.try_mint(&admin, &alice, &0),
        Err(Ok(sdk_error(Error::InvalidAmount)))
    );

    assert_eq!(
        client.try_transfer(&alice, &bob, &-1),
        Err(Ok(sdk_error(Error::InvalidAmount)))
    );
}

#[test]
fn test_attacker_cannot_whitelist() {
    let env = Env::default();
    let (id, _) = setup(&env);
    let client = RwaLaunchpadClient::new(&env, &id);
    let attacker = Address::generate(&env);
    let investor = Address::generate(&env);

    // Autorizar solamente al atacante.
    env.mock_auths(&[MockAuth {
        address: &attacker,
        invoke: &MockAuthInvoke {
            contract: &id,
            fn_name: "set_whitelist",
            args: (attacker.clone(), investor.clone(), true).into_val(&env),
            sub_invokes: &[],
        },
    }]);

    assert!(client
        .try_set_whitelist(&attacker, &investor, &true)
        .is_err());

    assert!(!client.is_whitelisted(&investor));
}

#[test]
fn test_attacker_cannot_mint() {
    let env = Env::default();
    let (id, _) = setup(&env);
    let client = RwaLaunchpadClient::new(&env, &id);
    let attacker = Address::generate(&env);
    let recipient = Address::generate(&env);

    env.mock_auths(&[MockAuth {
        address: &attacker,
        invoke: &MockAuthInvoke {
            contract: &id,
            fn_name: "mint",
            args: (attacker.clone(), recipient.clone(), 500i128).into_val(&env),
            sub_invokes: &[],
        },
    }]);

    assert!(client.try_mint(&attacker, &recipient, &500).is_err());
    assert_eq!(client.balance(&recipient), 0);
}

#[test]
fn test_wrong_admin_argument_rejected_even_with_mocked_signatures() {
    let env = Env::default();
    let (id, _) = setup(&env);
    let client = RwaLaunchpadClient::new(&env, &id);
    let wrong_admin = Address::generate(&env);
    let investor = Address::generate(&env);

    assert_eq!(
        client.try_set_whitelist(&wrong_admin, &investor, &true),
        Err(Ok(sdk_error(Error::Unauthorized)))
    );
}

#[test]
fn test_whitelist_approval_and_revocation() {
    let env = Env::default();
    let (id, admin) = setup(&env);
    let client = RwaLaunchpadClient::new(&env, &id);
    let investor = Address::generate(&env);

    assert!(!client.is_whitelisted(&investor));

    client.set_whitelist(&admin, &investor, &true);
    assert!(client.is_whitelisted(&investor));
    client.check_access(&investor);

    client.set_whitelist(&admin, &investor, &false);
    assert!(!client.is_whitelisted(&investor));

    // check_access declara Result<(), Error>:
    // aqui se compara con nuestro enum directamente.
    assert_eq!(
        client.try_check_access(&investor),
        Err(Ok(Error::NotWhitelisted))
    );
}

#[test]
fn test_queries_and_mint_do_not_consume_slots() {
    let env = Env::default();
    let (id, admin) = setup(&env);
    let client = RwaLaunchpadClient::new(&env, &id);
    let investor = Address::generate(&env);

    client.set_whitelist(&admin, &investor, &true);
    client.mint(&admin, &investor, &100);

    for _ in 0..15 {
        client.check_access(&investor);
    }

    assert_eq!(client.remaining_slots(), 10);
}

#[test]
fn test_ten_internal_slot_consumptions_then_rejection() {
    let env = Env::default();
    let (id, admin) = setup(&env);
    let client = RwaLaunchpadClient::new(&env, &id);
    let investor = Address::generate(&env);

    client.set_whitelist(&admin, &investor, &true);

    // Prueba del contador interno, sin pagos.
    for used in 1u32..=10 {
        let result = env.as_contract(&id, || {
            RwaLaunchpad::consume_investment_slot(&env, &investor)
        });

        assert_eq!(result, Ok(()));
        assert_eq!(client.remaining_slots(), 10 - used);
    }

    let eleventh = env.as_contract(&id, || {
        RwaLaunchpad::consume_investment_slot(&env, &investor)
    });

    assert_eq!(eleventh, Err(Error::NoSlotsAvailable));
    assert_eq!(client.remaining_slots(), 0);

    assert_eq!(
        client.try_check_access(&investor),
        Err(Ok(Error::NoSlotsAvailable))
    );
}

#[test]
fn test_rejected_access_does_not_consume_slot() {
    let env = Env::default();
    let (id, _) = setup(&env);
    let client = RwaLaunchpadClient::new(&env, &id);
    let investor = Address::generate(&env);

    let result = env.as_contract(&id, || {
        RwaLaunchpad::consume_investment_slot(&env, &investor)
    });

    assert_eq!(result, Err(Error::NotWhitelisted));
    assert_eq!(client.remaining_slots(), 10);
}

#[test]
fn test_paused_contract_rejects_access() {
    let env = Env::default();
    let (id, admin) = setup(&env);
    let client = RwaLaunchpadClient::new(&env, &id);
    let investor = Address::generate(&env);

    client.set_whitelist(&admin, &investor, &true);

    env.as_contract(&id, || {
        let mut asset = RwaLaunchpad::read_asset(&env);
        asset.paused = true;

        env.storage().instance().set(&DataKey::AssetInfo, &asset);
    });

    assert_eq!(client.try_check_access(&investor), Err(Ok(Error::Paused)));

    let result = env.as_contract(&id, || {
        RwaLaunchpad::consume_investment_slot(&env, &investor)
    });

    assert_eq!(result, Err(Error::Paused));
    assert_eq!(client.remaining_slots(), 10);
}

#[test]
fn test_cannot_initialize_twice() {
    let env = Env::default();
    let (id, _) = setup(&env);
    let client = RwaLaunchpadClient::new(&env, &id);
    let another_admin = Address::generate(&env);

    assert_eq!(
        client.try_initialize(&another_admin, &sample_asset(&env)),
        Err(Ok(sdk_error(Error::AlreadyInitialized)))
    );
}
