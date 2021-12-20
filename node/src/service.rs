//! Service and ServiceFactory implementation. Specialized wrapper over substrate service.
use node_template_runtime::{self, opaque::Block, RuntimeApi};
use sc_client_api::ExecutorProvider;
use sc_consensus_aura::{ImportQueueParams, SlotProportion, StartAuraParams};
pub use sc_executor::NativeElseWasmExecutor;
use sc_keystore::LocalKeystore;
use sc_service::{error::Error as ServiceError, Configuration, TaskManager};
use sc_telemetry::{Telemetry, TelemetryWorker};
use sp_consensus::SlotData;
use std::{sync::Arc, time::Duration};
pub use sc_rpc_api::DenyUnsafe;
use sc_finality_grandpa::{
	FinalityProofProvider, GrandpaJustificationStream, SharedAuthoritySet, SharedVoterState,
};
use node_template_runtime::{Hash, BlockNumber};
use sp_runtime::{generic, traits::Block as BlockT, SaturatedConversion};

// Our native executor instance.
pub struct ExecutorDispatch;

impl sc_executor::NativeExecutionDispatch for ExecutorDispatch {
	/// Only enable the benchmarking host functions when we actually want to benchmark.
	#[cfg(feature = "runtime-benchmarks")]
	type ExtendHostFunctions = frame_benchmarking::benchmarking::HostFunctions;
	/// Otherwise we only use the default Substrate host functions.
	#[cfg(not(feature = "runtime-benchmarks"))]
	type ExtendHostFunctions = ();

	fn dispatch(method: &str, data: &[u8]) -> Option<Vec<u8>> {
		node_template_runtime::api::dispatch(method, data)
	}

	fn native_version() -> sc_executor::NativeVersion {
		node_template_runtime::native_version()
	}
}

// A shared client instance.
type FullClient = sc_service::TFullClient<Block, RuntimeApi, NativeElseWasmExecutor<ExecutorDispatch>>;
// A shared backend instance.
type FullBackend = sc_service::TFullBackend<Block>;
// A chain selection algorithm instance
type FullSelectChain = sc_consensus::LongestChain<FullBackend, Block>;
type FullGrandpaBlockImport = sc_finality_grandpa::GrandpaBlockImport<FullBackend, Block, FullClient, FullSelectChain>;
// Everything else that needs to be passed into the main build function.
type Other = (
	impl Fn(
		crate::rpc::DenyUnsafe,
		sc_rpc::SubscriptionTaskExecutor,
	) -> Result<crate::rpc::IoHandler, sc_service::Error>,
	(
		sc_consensus_babe::BabeBlockImport<Block, FullClient, FullGrandpaBlockImport>,
		sc_finality_grandpa::LinkHalf<Block, FullClient, FullSelectChain>,
		sc_consensus_babe::BabeLink<Block>,
	),
	sc_finality_grandpa::SharedVoterState,
	Option<Telemetry>,
);

type PartialComponents = sc_service::PartialComponents<
	FullClient,
	FullBackend,
	FullSelectChain,
	sc_consensus::DefaultImportQueue<Block, FullClient>,
	sc_transaction_pool::FullPool<Block, FullClient>,
	Other,
>;

pub fn new_partial(
	config: &Configuration,
) -> Result<PartialComponents, ServiceError> {
	if config.keystore_remote.is_some() {
		return Err(ServiceError::Other(format!("Remote Keystores are not supported.")))
	}

	let telemetry = config
		.telemetry_endpoints
		.clone()
		.filter(|x| !x.is_empty())
		.map(|endpoints| -> Result<_, sc_telemetry::Error> {
			let worker = TelemetryWorker::new(16)?;
			let telemetry = worker.handle().new_telemetry(endpoints);
			Ok((worker, telemetry))
		})
		.transpose()?;

	let executor = NativeElseWasmExecutor::<ExecutorDispatch>::new(
		config.wasm_method,
		config.default_heap_pages,
		config.max_runtime_instances,
	);

	let (client, backend, keystore_container, task_manager) =
		sc_service::new_full_parts::<Block, RuntimeApi, _>(
			&config,
			telemetry.as_ref().map(|(_, telemetry)| telemetry.handle()),
			executor,
		)?;
	let client = Arc::new(client);

	let telemetry = telemetry.map(|(worker, telemetry)| {
		task_manager.spawn_handle().spawn("telemetry", worker.run());
		telemetry
	});

	let select_chain = sc_consensus::LongestChain::new(backend.clone());

	let transaction_pool = sc_transaction_pool::BasicPool::new_full(
		config.transaction_pool.clone(),
		config.role.is_authority().into(),
		config.prometheus_registry(),
		task_manager.spawn_essential_handle(),
		client.clone(),
	);

	let (grandpa_block_import, grandpa_link) = sc_finality_grandpa::block_import(
		client.clone(),
		&(client.clone() as Arc<_>),
		select_chain.clone(),
		telemetry.as_ref().map(|x| x.handle()),
	)?;
	let justification_import = grandpa_block_import.clone();
	let (block_import, babe_link) = sc_consensus_babe::block_import(
		sc_consensus_babe::Config::get_or_compute(&*client)?,
		grandpa_block_import,
		client.clone(),
	)?;

	let slot_duration =  babe_link.config().slot_duration();

	let import_queue = sc_consensus_babe::import_queue(
		babe_link.clone(),
		block_import.clone(),
		Some(Box::new(justification_import)),
		client.clone(),
		select_chain.clone(),
		move |_, ()| async move {
				let timestamp = sp_timestamp::InherentDataProvider::from_system_time();

				let slot =
					sp_consensus_babe::inherents::InherentDataProvider::from_timestamp_and_duration(
						*timestamp,
						slot_duration,
					);
				
				let uncles =
					sp_authorship::InherentDataProvider::<<Block as BlockT>::Header>::check_inherents();
	
				Ok((timestamp, slot, uncles))
		},
		&task_manager.spawn_essential_handle(),
		config.prometheus_registry(),
		sp_consensus::CanAuthorWithNativeVersion::new(client.executor().clone()),
		telemetry.as_ref().map(|x| x.handle()),
	)?;

	let import_setup = (block_import, grandpa_link, babe_link);

	let (rpc_extensions_builder, rpc_setup) = {
		let (_, grandpa_link, babe_link) = &import_setup;

		let justification_stream = grandpa_link.justification_stream();
		let shared_authority_set = grandpa_link.shared_authority_set().clone();
		let shared_voter_state = sc_finality_grandpa::SharedVoterState::empty();
		let rpc_setup = shared_voter_state.clone();

		let finality_proof_provider = sc_finality_grandpa::FinalityProofProvider::new_for_service(
			backend.clone(),
			Some(shared_authority_set.clone()),
		);

		let babe_config = babe_link.config().clone();
		let shared_epoch_changes = babe_link.epoch_changes().clone();

		let client = client.clone();
		let pool = transaction_pool.clone();
		let select_chain = select_chain.clone();
		let keystore = keystore_container.sync_keystore();
		let chain_spec = config.chain_spec.cloned_box();

		let rpc_extensions_builder = move |deny_unsafe, subscription_executor| {
			let deps = crate::rpc::FullDeps {
				client: client.clone(),
				pool: pool.clone(),
				select_chain: select_chain.clone(),
				chain_spec: chain_spec.cloned_box(),
				deny_unsafe,
				babe: crate::rpc::BabeDeps {
					babe_config: babe_config.clone(),
					shared_epoch_changes: shared_epoch_changes.clone(),
					keystore: keystore.clone(),
				},
				grandpa: crate::rpc::GrandpaDeps {
					shared_voter_state: shared_voter_state.clone(),
					shared_authority_set: shared_authority_set.clone(),
					justification_stream: justification_stream.clone(),
					subscription_executor,
					finality_provider: finality_proof_provider.clone(),
				},
			};

			crate::rpc::create_full(deps)
		};

		(rpc_extensions_builder, rpc_setup)
	};

	Ok(sc_service::PartialComponents {
		client,
		backend,
		task_manager,
		keystore_container,
		select_chain,
		import_queue,
		transaction_pool,
		other: (rpc_extensions_builder, import_setup, rpc_setup, telemetry),
	})
}

fn remote_keystore(_url: &String) -> Result<Arc<LocalKeystore>, &'static str> {
	// FIXME: here would the concrete keystore be built,
	//        must return a concrete type (NOT `LocalKeystore`) that
	//        implements `CryptoStore` and `SyncCryptoStore`
	Err("Remote Keystore not supported.")
}

/// Builds a new service for a full client.
pub fn new_full(mut config: Configuration) -> Result<TaskManager, ServiceError> {
	// let sc_service::PartialComponents {
	// 	client,
	// 	backend,
	// 	mut task_manager,
	// 	import_queue,
	// 	mut keystore_container,
	// 	select_chain,
	// 	transaction_pool,
	// 	other: (block_import, grandpa_link, mut telemetry),
	// } = new_partial(&config)?;

	// if let Some(url) = &config.keystore_remote {
	// 	match remote_keystore(url) {
	// 		Ok(k) => keystore_container.set_remote_keystore(k),
	// 		Err(e) =>
	// 			return Err(ServiceError::Other(format!(
	// 				"Error hooking up remote keystore for {}: {}",
	// 				url, e
	// 			))),
	// 	};
	// }

	// config.network.extra_sets.push(sc_finality_grandpa::grandpa_peers_set_config());
	// let warp_sync = Arc::new(sc_finality_grandpa::warp_proof::NetworkProvider::new(
	// 	backend.clone(),
	// 	grandpa_link.shared_authority_set().clone(),
	// 	Vec::default(),
	// ));

	// let (network, system_rpc_tx, network_starter) =
	// 	sc_service::build_network(sc_service::BuildNetworkParams {
	// 		config: &config,
	// 		client: client.clone(),
	// 		transaction_pool: transaction_pool.clone(),
	// 		spawn_handle: task_manager.spawn_handle(),
	// 		import_queue,
	// 		on_demand: None,
	// 		block_announce_validator_builder: None,
	// 		warp_sync: Some(warp_sync),
	// 	})?;

	// if config.offchain_worker.enabled {
	// 	sc_service::build_offchain_workers(
	// 		&config,
	// 		task_manager.spawn_handle(),
	// 		client.clone(),
	// 		network.clone(),
	// 	);
	// }

	// let role = config.role.clone();
	// let force_authoring = config.force_authoring;
	// let backoff_authoring_blocks: Option<()> = None;
	// let name = config.network.node_name.clone();
	// let enable_grandpa = !config.disable_grandpa;
	// let prometheus_registry = config.prometheus_registry().cloned();

	// let rpc_extensions_builder = {
	// 	let client = client.clone();
	// 	let pool = transaction_pool.clone();

	// 	Box::new(move |deny_unsafe, _| {
	// 		let deps =
	// 			crate::rpc::FullDeps { client: client.clone(), pool: pool.clone(), deny_unsafe };

	// 		Ok(crate::rpc::create_full(deps))
	// 	})
	// };

	// let _rpc_handlers = sc_service::spawn_tasks(sc_service::SpawnTasksParams {
	// 	network: network.clone(),
	// 	client: client.clone(),
	// 	keystore: keystore_container.sync_keystore(),
	// 	task_manager: &mut task_manager,
	// 	transaction_pool: transaction_pool.clone(),
	// 	rpc_extensions_builder,
	// 	on_demand: None,
	// 	remote_blockchain: None,
	// 	backend,
	// 	system_rpc_tx,
	// 	config,
	// 	telemetry: telemetry.as_mut(),
	// })?;

	// if role.is_authority() {
	// 	let proposer_factory = sc_basic_authorship::ProposerFactory::new(
	// 		task_manager.spawn_handle(),
	// 		client.clone(),
	// 		transaction_pool,
	// 		prometheus_registry.as_ref(),
	// 		telemetry.as_ref().map(|x| x.handle()),
	// 	);

	// 	let can_author_with =
	// 		sp_consensus::CanAuthorWithNativeVersion::new(client.executor().clone());

	// 	let slot_duration = sc_consensus_aura::slot_duration(&*client)?;
	// 	let raw_slot_duration = slot_duration.slot_duration();

	// 	let aura = sc_consensus_aura::start_aura::<AuraPair, _, _, _, _, _, _, _, _, _, _, _>(
	// 		StartAuraParams {
	// 			slot_duration,
	// 			client: client.clone(),
	// 			select_chain,
	// 			block_import,
	// 			proposer_factory,
	// 			create_inherent_data_providers: move |_, ()| async move {
	// 				let timestamp = sp_timestamp::InherentDataProvider::from_system_time();

	// 				let slot =
	// 					sp_consensus_aura::inherents::InherentDataProvider::from_timestamp_and_duration(
	// 						*timestamp,
	// 						raw_slot_duration,
	// 					);

	// 				Ok((timestamp, slot))
	// 			},
	// 			force_authoring,
	// 			backoff_authoring_blocks,
	// 			keystore: keystore_container.sync_keystore(),
	// 			can_author_with,
	// 			sync_oracle: network.clone(),
	// 			justification_sync_link: network.clone(),
	// 			block_proposal_slot_portion: SlotProportion::new(2f32 / 3f32),
	// 			max_block_proposal_slot_portion: None,
	// 			telemetry: telemetry.as_ref().map(|x| x.handle()),
	// 		},
	// 	)?;

	// 	// the AURA authoring task is considered essential, i.e. if it
	// 	// fails we take down the service with it.
	// 	task_manager.spawn_essential_handle().spawn_blocking("aura", aura);
	// }

	// // if the node isn't actively participating in consensus then it doesn't
	// // need a keystore, regardless of which protocol we use below.
	// let keystore =
	// 	if role.is_authority() { Some(keystore_container.sync_keystore()) } else { None };

	// let grandpa_config = sc_finality_grandpa::Config {
	// 	// FIXME #1578 make this available through chainspec
	// 	gossip_duration: Duration::from_millis(333),
	// 	justification_period: 512,
	// 	name: Some(name),
	// 	observer_enabled: false,
	// 	keystore,
	// 	local_role: role,
	// 	telemetry: telemetry.as_ref().map(|x| x.handle()),
	// };

	// if enable_grandpa {
	// 	// start the full GRANDPA voter
	// 	// NOTE: non-authorities could run the GRANDPA observer protocol, but at
	// 	// this point the full voter should provide better guarantees of block
	// 	// and vote data availability than the observer. The observer has not
	// 	// been tested extensively yet and having most nodes in a network run it
	// 	// could lead to finality stalls.
	// 	let grandpa_config = sc_finality_grandpa::GrandpaParams {
	// 		config: grandpa_config,
	// 		link: grandpa_link,
	// 		network,
	// 		voting_rule: sc_finality_grandpa::VotingRulesBuilder::default().build(),
	// 		prometheus_registry,
	// 		shared_voter_state: SharedVoterState::empty(),
	// 		telemetry: telemetry.as_ref().map(|x| x.handle()),
	// 	};

	// 	// the GRANDPA voter task is considered infallible, i.e.
	// 	// if it fails we take down the service with it.
	// 	task_manager.spawn_essential_handle().spawn_blocking(
	// 		"grandpa-voter",
	// 		sc_finality_grandpa::run_grandpa_voter(grandpa_config)?,
	// 	);
	// }

	// network_starter.start_network();
	// Ok(task_manager)
	todo!()
}

// /// Instantiate all Full RPC extensions.
// pub fn create_full<C, P, SC, B>(
// 	deps: FullDeps<C, P, SC, B>,
// ) -> Result<jsonrpc_core::IoHandler<sc_rpc_api::Metadata>, Box<dyn std::error::Error + Send + Sync>>
// where
// 	C: ProvideRuntimeApi<Block>
// 		+ HeaderBackend<Block>
// 		+ AuxStore
// 		+ HeaderMetadata<Block, Error = BlockChainError>
// 		+ Sync
// 		+ Send
// 		+ 'static,
// 	C::Api: substrate_frame_rpc_system::AccountNonceApi<Block, AccountId, Index>,
// 	C::Api: pallet_contracts_rpc::ContractsRuntimeApi<Block, AccountId, Balance, BlockNumber, Hash>,
// 	C::Api: pallet_mmr_rpc::MmrRuntimeApi<Block, <Block as sp_runtime::traits::Block>::Hash>,
// 	C::Api: pallet_transaction_payment_rpc::TransactionPaymentRuntimeApi<Block, Balance>,
// 	C::Api: BabeApi<Block>,
// 	C::Api: BlockBuilder<Block>,
// 	P: TransactionPool + 'static,
// 	SC: SelectChain<Block> + 'static,
// 	B: sc_client_api::Backend<Block> + Send + Sync + 'static,
// 	B::State: sc_client_api::backend::StateBackend<sp_runtime::traits::HashFor<Block>>,
// {
// 	use pallet_contracts_rpc::{Contracts, ContractsApi};
// 	use pallet_transaction_payment_rpc::{TransactionPayment, TransactionPaymentApi};
// 	use substrate_frame_rpc_system::{FullSystem, SystemApi};

// 	let mut io = jsonrpc_core::IoHandler::default();
// 	let FullDeps { client, pool, select_chain, chain_spec, deny_unsafe, babe, grandpa } = deps;

// 	let BabeDeps { keystore, babe_config, shared_epoch_changes } = babe;
// 	let GrandpaDeps {
// 		shared_voter_state,
// 		shared_authority_set,
// 		justification_stream,
// 		subscription_executor,
// 		finality_provider,
// 	} = grandpa;

// 	io.extend_with(SystemApi::to_delegate(FullSystem::new(client.clone(), pool, deny_unsafe)));
// 	// Making synchronous calls in light client freezes the browser currently,
// 	// more context: https://github.com/paritytech/substrate/pull/3480
// 	// These RPCs should use an asynchronous caller instead.
// 	io.extend_with(ContractsApi::to_delegate(Contracts::new(client.clone())));
// 	io.extend_with(MmrApi::to_delegate(Mmr::new(client.clone())));
// 	io.extend_with(TransactionPaymentApi::to_delegate(TransactionPayment::new(client.clone())));
// 	io.extend_with(sc_consensus_babe_rpc::BabeApi::to_delegate(BabeRpcHandler::new(
// 		client.clone(),
// 		shared_epoch_changes.clone(),
// 		keystore,
// 		babe_config,
// 		select_chain,
// 		deny_unsafe,
// 	)));
// 	io.extend_with(sc_finality_grandpa_rpc::GrandpaApi::to_delegate(GrandpaRpcHandler::new(
// 		shared_authority_set.clone(),
// 		shared_voter_state,
// 		justification_stream,
// 		subscription_executor,
// 		finality_provider,
// 	)));

// 	io.extend_with(sc_sync_state_rpc::SyncStateRpcApi::to_delegate(
// 		sc_sync_state_rpc::SyncStateRpcHandler::new(
// 			chain_spec,
// 			client,
// 			shared_authority_set,
// 			shared_epoch_changes,
// 			deny_unsafe,
// 		)?,
// 	));

// 	Ok(io)
// }
