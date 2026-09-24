#![no_std]

use soroban_sdk::{
    contract, contracterror, contractevent, contractimpl, contracttype, panic_with_error, Address,
    Env, Symbol,
};

const MAX_SLOTS: u32 = 10;

#[contracttype]
pub enum DataKey {
    Admin,
    AssetInfo,
    Balance(Address),
    Whitelisted(Address),
    UsedSlots,
}

#[contracttype]
pub struct AssetInfo {
    pub name: Symbol,
    pub total_supply: i128,
    pub price_per_unit: i128,
    pub payment_token: Address,
    pub paused: bool,
}

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum Error {
    NotInitialized = 1,
    AlreadyInitialized = 2,
    InsufficientBalance = 3,
    InvalidAmount = 4,
    NotWhitelisted = 5,
    Paused = 6,
    Unauthorized = 7,
    NoSlotsAvailable = 8,
    Overflow = 9,
}

#[contractevent(topics = ["mint"])]
pub struct MintEvent {
    #[topic]
    pub to: Address,
    pub amount: i128,
}

#[contractevent(topics = ["transfer"])]
pub struct TransferEvent {
    #[topic]
    pub from: Address,
    #[topic]
    pub to: Address,
    pub amount: i128,
}

#[contractevent(topics = ["whitelist"])]
pub struct WhitelistEvent {
    #[topic]
    pub investor: Address,
    pub approved: bool,
}

#[contract]
pub struct RwaLaunchpad;

impl RwaLaunchpad {
    fn require_initialized(env: &Env) {
        if !env.storage().instance().has(&DataKey::Admin) {
            panic_with_error!(env, Error::NotInitialized);
        }
    }

    fn read_asset(env: &Env) -> AssetInfo {
        env.storage().instance().get(&DataKey::AssetInfo).unwrap()
    }

    fn require_admin(env: &Env, caller: &Address) {
        Self::require_initialized(env);

        let admin: Address = env.storage().instance().get(&DataKey::Admin).unwrap();

        admin.require_auth();

        if caller != &admin {
            panic_with_error!(env, Error::Unauthorized);
        }
    }

    fn require_not_paused(env: &Env) {
        if Self::read_asset(env).paused {
            panic_with_error!(env, Error::Paused);
        }
    }

    fn require_positive(env: &Env, amount: i128) {
        if amount <= 0 {
            panic_with_error!(env, Error::InvalidAmount);
        }
    }

    fn checked_add(env: &Env, left: i128, right: i128) -> i128 {
        match left.checked_add(right) {
            Some(value) => value,
            None => panic_with_error!(env, Error::Overflow),
        }
    }

    fn read_balance(env: &Env, owner: &Address) -> i128 {
        env.storage()
            .persistent()
            .get(&DataKey::Balance(owner.clone()))
            .unwrap_or(0)
    }

    fn write_balance(env: &Env, owner: &Address, amount: i128) {
        env.storage()
            .persistent()
            .set(&DataKey::Balance(owner.clone()), &amount);
    }

    fn read_whitelist(env: &Env, investor: &Address) -> bool {
        env.storage()
            .persistent()
            .get(&DataKey::Whitelisted(investor.clone()))
            .unwrap_or(false)
    }

    fn used_slots(env: &Env) -> u32 {
        env.storage()
            .instance()
            .get(&DataKey::UsedSlots)
            .unwrap_or(0)
    }

    // Solo consulta: no consume cupos.
    fn check_variation_gate(env: &Env, _investor: &Address) -> Result<(), Error> {
        if Self::used_slots(env) >= MAX_SLOTS {
            return Err(Error::NoSlotsAvailable);
        }

        Ok(())
    }

    fn validate_access(env: &Env, investor: &Address) -> Result<(), Error> {
        if !env.storage().instance().has(&DataKey::Admin) {
            return Err(Error::NotInitialized);
        }

        if Self::read_asset(env).paused {
            return Err(Error::Paused);
        }

        if !Self::read_whitelist(env, investor) {
            return Err(Error::NotWhitelisted);
        }

        Self::check_variation_gate(env, investor)
    }

    // Funcion interna, NO disponible como operacion publica.
    //
    // Dia 3: invocarla una sola vez dentro de invest(), despues
    // de validar y completar el pago y la emision de tokens.
    // Si la transaccion falla, sus cambios deben revertirse.
    //
    // Hasta incorporar invest(), solo se utiliza en las pruebas.
    #[cfg_attr(not(test), allow(dead_code))]
    fn consume_investment_slot(env: &Env, investor: &Address) -> Result<(), Error> {
        Self::validate_access(env, investor)?;

        let next = Self::used_slots(env) + 1;
        env.storage().instance().set(&DataKey::UsedSlots, &next);

        Ok(())
    }
}

#[contractimpl]
impl RwaLaunchpad {
    pub fn initialize(env: Env, admin: Address, asset: AssetInfo) {
        if env.storage().instance().has(&DataKey::Admin) {
            panic_with_error!(&env, Error::AlreadyInitialized);
        }

        admin.require_auth();

        env.storage().instance().set(&DataKey::Admin, &admin);
        env.storage().instance().set(&DataKey::AssetInfo, &asset);
        env.storage().instance().set(&DataKey::UsedSlots, &0u32);
    }

    pub fn balance(env: Env, id: Address) -> i128 {
        Self::require_initialized(&env);
        Self::read_balance(&env, &id)
    }

    pub fn mint(env: Env, admin: Address, to: Address, amount: i128) {
        Self::require_admin(&env, &admin);
        Self::require_not_paused(&env);
        Self::require_positive(&env, amount);

        let previous = Self::read_balance(&env, &to);
        let next = Self::checked_add(&env, previous, amount);
        Self::write_balance(&env, &to, next);

        MintEvent { to, amount }.publish(&env);
    }

    pub fn transfer(env: Env, from: Address, to: Address, amount: i128) {
        Self::require_initialized(&env);
        from.require_auth();
        Self::require_not_paused(&env);
        Self::require_positive(&env, amount);

        let available = Self::read_balance(&env, &from);
        if available < amount {
            panic_with_error!(&env, Error::InsufficientBalance);
        }

        if from != to {
            let destination = Self::read_balance(&env, &to);
            let next = Self::checked_add(&env, destination, amount);

            Self::write_balance(&env, &from, available - amount);
            Self::write_balance(&env, &to, next);
        }

        TransferEvent { from, to, amount }.publish(&env);
    }

    pub fn set_whitelist(env: Env, admin: Address, investor: Address, approved: bool) {
        Self::require_admin(&env, &admin);

        env.storage()
            .persistent()
            .set(&DataKey::Whitelisted(investor.clone()), &approved);

        WhitelistEvent { investor, approved }.publish(&env);
    }

    pub fn is_whitelisted(env: Env, investor: Address) -> bool {
        Self::require_initialized(&env);
        Self::read_whitelist(&env, &investor)
    }

    pub fn remaining_slots(env: Env) -> u32 {
        Self::require_initialized(&env);
        MAX_SLOTS.saturating_sub(Self::used_slots(&env))
    }

    // Consulta publica; no reserva ni consume un cupo.
    pub fn check_access(env: Env, investor: Address) -> Result<(), Error> {
        Self::validate_access(&env, &investor)
    }
}

#[cfg(test)]
mod test;
