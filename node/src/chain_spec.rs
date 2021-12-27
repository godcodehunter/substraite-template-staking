use frame_benchmarking::frame_support::traits::tokens::Balance;
use node_template_runtime::{
	AccountId, SessionConfig, StakingConfig, BabeConfig, BalancesConfig, GenesisConfig, GrandpaConfig, Signature, SudoConfig,
	SystemConfig, WASM_BINARY, StakerStatus, AuthorityDiscoveryConfig,
};
use sc_service::ChainType;
use sp_consensus_babe::AuthorityId as BabeId;
use sp_core::{sr25519, Pair, Public};
use sp_finality_grandpa::AuthorityId as GrandpaId;
use sp_runtime::{traits::{IdentifyAccount, Verify}, Perbill};
use node_template_runtime::opaque::SessionKeys;

// The URL for the telemetry server.
// const STAGING_TELEMETRY_URL: &str = "wss://telemetry.polkadot.io/submit/";

/// Specialized `ChainSpec`. This is a specialization of the general Substrate ChainSpec type.
pub type ChainSpec = sc_service::GenericChainSpec<GenesisConfig>;

/// Generate a crypto pair from seed.
pub fn get_from_seed<TPublic: Public>(seed: &str) -> <TPublic::Pair as Pair>::Public {
	TPublic::Pair::from_string(&format!("//{}", seed), None)
		.expect("static values are valid; qed")
		.public()
}

type AccountPublic = <Signature as Verify>::Signer;

/// Generate an account ID from seed.
pub fn get_account_id_from_seed<TPublic: Public>(seed: &str) -> AccountId
where
	AccountPublic: From<<TPublic::Pair as Pair>::Public>,
{
	AccountPublic::from(get_from_seed::<TPublic>(seed)).into_account()
}


#[derive(Clone)]
struct StashConfig {
	stash: AccountId,
	controller: AccountId,
	balance: u128,
}

impl StashConfig {
    fn new(stash: AccountId, controller: AccountId, balance: u128) -> Self { 
		Self { stash, controller, balance }
	}
}

#[derive(Clone)]
struct AccountConfig {
	id: AccountId,
	balance: u128,
}

impl AccountConfig {
    fn new_from_seed(str: &str, balance: u128) -> Self { 
		Self { id: get_account_id_from_seed::<sr25519::Public>(str), balance } 
	}
}

/// Configure initial storage state for FRAME modules.
fn testnet_genesis(
	wasm_binary: &[u8],
	initial_authorities: Vec<(AccountId, AccountId, BabeId, GrandpaId)>,
	root_key: AccountId,
	endowed_accounts: Vec<AccountConfig>,
	stakers: Vec<StashConfig>,
) -> GenesisConfig {

	GenesisConfig {
		system: SystemConfig {
			code: wasm_binary.to_vec(),
			changes_trie_config: None,
		},
		sudo: SudoConfig {
			key: root_key,
		},
		balances: BalancesConfig {
			balances: endowed_accounts
				.iter()
				.cloned()
				.map(|i| (i.id, i.balance))
				.collect(),
		},
		authority_discovery: AuthorityDiscoveryConfig { keys: vec![] },
		// Babe, Grandpa authorities come from Session pallet, 
		// so it is should not be installed manually
		babe: BabeConfig {
			authorities: vec![],
			epoch_config: Some(node_template_runtime::BABE_GENESIS_EPOCH_CONFIG),
		},
		grandpa: GrandpaConfig {
			authorities: vec![],
		},
		session: SessionConfig {
			// Stash, controller, session-key
    		keys: initial_authorities
			.iter()
			.map(|i| {
				(
					i.0.clone(), 
					i.1.clone(), 
					SessionKeys {
						babe: i.2.clone(),
						grandpa: i.3.clone(),
    				},
				)
			})
			.collect::<Vec<_>>(), 
		},
		staking: StakingConfig {
			stakers: stakers
				.iter()
				.map(|i| (i.stash.clone(), i.controller.clone(), i.balance, StakerStatus::Validator))
				.collect(),
			validator_count: stakers.len() as u32,
			minimum_validator_count: stakers.len() as u32,
			slash_reward_fraction: Perbill::from_percent(10),
			invulnerables: stakers.iter().map(|i| i.stash.clone()).collect(),
			..Default::default()
		}, 
		transaction_payment: Default::default(),
	}
}

pub fn development_config() -> Result<ChainSpec, String> {
	let wasm_binary = WASM_BINARY.ok_or_else(|| "Development wasm not available".to_string())?;

	let alice = AccountConfig::new_from_seed("Alice", 1 << 60);
	let bob = AccountConfig::new_from_seed("Bob", 1 << 60);
	let charlie = AccountConfig::new_from_seed("Charlie", 1 << 60);
	let dave = AccountConfig::new_from_seed("Dave", 1 << 60);
	let eve = AccountConfig::new_from_seed("Eve", 1 << 60);
	let ferdie = AccountConfig::new_from_seed("Ferdie", 1 << 60);
	let alice_stash = AccountConfig::new_from_seed("Alice//stash", 1 << 60);
	let bob_stash = AccountConfig::new_from_seed("Bob//stash", 1 << 60);
	let charlie_stash = AccountConfig::new_from_seed("Charlie//stash", 1 << 60);
	let dave_stash = AccountConfig::new_from_seed("Dave//stash", 1 << 60);
	let eve_stash = AccountConfig::new_from_seed("Eve//stash", 1 << 60);
	let ferdie_stash = AccountConfig::new_from_seed("Ferdie//stash", 1 << 60);

	let alice_stake = StashConfig::new(alice_stash.id.clone(), alice.id.clone(), (1 << 60)/2);
	let bob_stake = StashConfig::new(bob_stash.id.clone(), bob.id.clone(), (1 << 60)/2);
	let charlie_stake = StashConfig::new(charlie_stash.id.clone(), charlie.id.clone(), (1 << 60)/2);
	let dave_stake = StashConfig::new(dave_stash.id.clone(), dave.id.clone(), (1 << 60)/2);
	let eve_stake= StashConfig::new(eve_stash.id.clone(), eve.id.clone(), (1 << 60)/2);
	let ferdie_stake = StashConfig::new(ferdie_stash.id.clone(), ferdie.id.clone(), (1 << 60)/2);

	Ok(ChainSpec::from_genesis(
		"Development",
		"dev",
		ChainType::Development,
		move || {
			testnet_genesis(
				wasm_binary,
				vec![
					(
						alice_stake.stash.clone(),
						alice_stake.controller.clone(),
						get_from_seed::<BabeId>("Alice"),
						get_from_seed::<GrandpaId>("Alice"),
					),
					(
						alice_stake.stash.clone(),
						alice_stake.controller.clone(),
						get_from_seed::<BabeId>("Bob"),
						get_from_seed::<GrandpaId>("Bob"),
					),
					(
						alice_stake.stash.clone(),
						alice_stake.controller.clone(),
						get_from_seed::<BabeId>("Charlie"),
						get_from_seed::<GrandpaId>("Charlie"),
					),
					(
						alice_stake.stash.clone(),
						alice_stake.controller.clone(),
						get_from_seed::<BabeId>("Dave"),
						get_from_seed::<GrandpaId>("Dave"),
					),
					(
						alice_stake.stash.clone(),
						alice_stake.controller.clone(),
						get_from_seed::<BabeId>("Eve"),
						get_from_seed::<GrandpaId>("Eve"),
					),
					(
						alice_stake.stash.clone(),
						alice_stake.controller.clone(),
						get_from_seed::<BabeId>("Ferdie"),
						get_from_seed::<GrandpaId>("Ferdie"),
					),
				],
				alice.id.clone(),
				vec![
					alice.clone(),
					bob.clone(),
					charlie.clone(),
					dave.clone(),
					eve.clone(),
					ferdie.clone(),
					alice_stash.clone(),
					bob_stash.clone(),
					charlie_stash.clone(),
					dave_stash.clone(),
					eve_stash.clone(),
					ferdie_stash.clone(),
				],
				vec![
					alice_stake.clone(),
					bob_stake.clone(),
					charlie_stake.clone(),
					dave_stake.clone(),
					eve_stake.clone(),
					ferdie_stake.clone(),
				],
			)
		},
		vec![],
		None,
		None,
		None,
		None,
	))
}

pub fn local_testnet_config() -> Result<ChainSpec, String> {
	let wasm_binary = WASM_BINARY.ok_or_else(|| "Development wasm not available".to_string())?;

	let alice = AccountConfig::new_from_seed("Alice", 1 << 60);
	let bob = AccountConfig::new_from_seed("Bob", 1 << 60);
	let charlie = AccountConfig::new_from_seed("Charlie", 1 << 60);
	let dave = AccountConfig::new_from_seed("Dave", 1 << 60);
	let eve = AccountConfig::new_from_seed("Eve", 1 << 60);
	let ferdie = AccountConfig::new_from_seed("Ferdie", 1 << 60);
	let alice_stash = AccountConfig::new_from_seed("Alice//stash", 1 << 60);
	let bob_stash = AccountConfig::new_from_seed("Bob//stash", 1 << 60);
	let charlie_stash = AccountConfig::new_from_seed("Charlie//stash", 1 << 60);
	let dave_stash = AccountConfig::new_from_seed("Dave//stash", 1 << 60);
	let eve_stash = AccountConfig::new_from_seed("Eve//stash", 1 << 60);
	let ferdie_stash = AccountConfig::new_from_seed("Ferdie//stash", 1 << 60);

	let alice_stake = StashConfig::new(alice_stash.id.clone(), alice.id.clone(), (1 << 60)/2);
	let bob_stake = StashConfig::new(bob_stash.id.clone(), bob.id.clone(), (1 << 60)/2);
	let charlie_stake = StashConfig::new(charlie_stash.id.clone(), charlie.id.clone(), (1 << 60)/2);
	let dave_stake = StashConfig::new(dave_stash.id.clone(), dave.id.clone(), (1 << 60)/2);
	let eve_stake= StashConfig::new(eve_stash.id.clone(), eve.id.clone(), (1 << 60)/2);
	let ferdie_stake = StashConfig::new(ferdie_stash.id.clone(), ferdie.id.clone(), (1 << 60)/2);

	Ok(ChainSpec::from_genesis(
		"Local Testnet",
		"local_testnet",
		ChainType::Local,
		move || {
			testnet_genesis(
				wasm_binary,
				vec![
					(
						alice_stake.stash.clone(),
						alice_stake.controller.clone(),
						get_from_seed::<BabeId>("Alice"),
						get_from_seed::<GrandpaId>("Alice"),
					),
					(
						alice_stake.stash.clone(),
						alice_stake.controller.clone(),
						get_from_seed::<BabeId>("Bob"),
						get_from_seed::<GrandpaId>("Bob"),
					),
					(
						alice_stake.stash.clone(),
						alice_stake.controller.clone(),
						get_from_seed::<BabeId>("Charlie"),
						get_from_seed::<GrandpaId>("Charlie"),
					),
					(
						alice_stake.stash.clone(),
						alice_stake.controller.clone(),
						get_from_seed::<BabeId>("Dave"),
						get_from_seed::<GrandpaId>("Dave"),
					),
					(
						alice_stake.stash.clone(),
						alice_stake.controller.clone(),
						get_from_seed::<BabeId>("Eve"),
						get_from_seed::<GrandpaId>("Eve"),
					),
					(
						alice_stake.stash.clone(),
						alice_stake.controller.clone(),
						get_from_seed::<BabeId>("Ferdie"),
						get_from_seed::<GrandpaId>("Ferdie"),
					),
				],
				alice.id.clone(),
				vec![
					alice.clone(),
					bob.clone(),
					charlie.clone(),
					dave.clone(),
					eve.clone(),
					ferdie.clone(),
					alice_stash.clone(),
					bob_stash.clone(),
					charlie_stash.clone(),
					dave_stash.clone(),
					eve_stash.clone(),
					ferdie_stash.clone(),
				],
				vec![
					alice_stake.clone(),
					bob_stake.clone(),
					charlie_stake.clone(),
					dave_stake.clone(),
					eve_stake.clone(),
					ferdie_stake.clone(),
				],
			)
		},
		vec![],
		None,
		None,
		None,
		None,
	))
}

