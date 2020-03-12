use crate::validator_pubkey_cache::ValidatorPubkeyCache;
use crate::{
    beacon_chain::{BLOCK_PROCESSING_CACHE_LOCK_TIMEOUT, VALIDATOR_PUBKEY_CACHE_LOCK_TIMEOUT},
    metrics, BeaconChain, BeaconChainError, BeaconChainTypes, BeaconSnapshot,
};
use parking_lot::RwLockReadGuard;
use state_processing::{
    block_signature_verifier::{
        BlockSignatureVerifier, Error as BlockSignatureVerifierError, G1Point,
    },
    per_slot_processing,
};
use std::borrow::Cow;
use types::{
    BeaconBlock, BeaconState, ChainSpec, CloneConfig, EthSpec, Hash256, RelativeEpoch,
    RelativeEpochError, SignedBeaconBlock, Slot,
};

pub enum BlockProcessingError {
    UnknownParent(Hash256),
    BlockIsEarlierThanParent,
    BeaconChainError(BeaconChainError),
}

pub struct ProposalSignatureVerifiedBlock<T: BeaconChainTypes> {
    block: SignedBeaconBlock<T::EthSpec>,
    block_root: Hash256,
    parent: BeaconSnapshot<T::EthSpec>,
}

pub struct FullySignatureVerifiedBlock<T: BeaconChainTypes> {
    block: SignedBeaconBlock<T::EthSpec>,
    block_root: Hash256,
    parent: Option<BeaconSnapshot<T::EthSpec>>,
}

pub struct ReadyToProcessBlock<T: BeaconChainTypes> {
    block: SignedBeaconBlock<T::EthSpec>,
    block_root: Hash256,
    parent: BeaconSnapshot<T::EthSpec>,
}

trait IntoReadyToProcessBlock {
    fn into_ready_to_process_block<T: BeaconChainTypes>(
        &self,
        chain: &BeaconChain<T>,
    ) -> Result<ReadyToProcessBlock<T>, BlockProcessingError>;
}

impl<T: BeaconChainTypes> ProposalSignatureVerifiedBlock<T> {
    pub fn new(
        block: SignedBeaconBlock<T::EthSpec>,
        chain: &BeaconChain<T>,
    ) -> Result<Self, BlockProcessingError> {
        let parent = load_parent(&block.message, chain)?;
        let block_root = block.canonical_root();

        todo!()
    }
}

/*
impl<T: BeaconChainTypes> SignatureVerifiedBlock<T> {
    pub fn new(
        block: SignedBeaconBlock<T::EthSpec>,
        chain: &BeaconChain<T>,
    ) -> Result<Self, SignatureVerificationError> {
        let parent = self
            .load_parent(chain)
            .map_err(SignatureVerificationError::BeaconChainError)?
            .ok_or_else(|| SignatureVerificationError::UnknownParent);
    }

    fn load_parent(
        &self,
        chain: &BeaconChain<T>,
    ) -> Result<Option<BeaconSnapshot<T::EthSpec>>, BeaconChainError> {
        let db_read_timer = metrics::start_timer(&metrics::BLOCK_PROCESSING_DB_READ);

        // Reject any block if its parent is not known to fork choice.
        //
        // A block that is not in fork choice is either:
        //
        //  - Not yet imported: we should reject this block because we should only import a child
        //  after its parent has been fully imported.
        //  - Pre-finalized: if the parent block is _prior_ to finalization, we should ignore it
        //  because it will revert finalization. Note that the finalized block is stored in fork
        //  choice, so we will not reject any child of the finalized block (this is relevant during
        //  genesis).
        if !chain.fork_choice.contains_block(&self.block.parent_root()) {
            return Ok(None);
        }

        // Load the parent block and state from disk, returning early if it's not available.
        let result = chain
            .snapshot_cache
            .try_write_for(BLOCK_PROCESSING_CACHE_LOCK_TIMEOUT)
            .and_then(|mut snapshot_cache| snapshot_cache.try_remove(self.block.parent_root()))
            .map(|snapshot| Ok(Some(snapshot)))
            .unwrap_or_else(|| {
                // Load the blocks parent block from the database, returning invalid if that block is not
                // found.
                //
                // We don't return a DBInconsistent error here since it's possible for a block to
                // exist in fork choice but not in the database yet. In such a case we simply
                // indicate that we don't yet know the parent.
                let parent_block =
                    if let Some(block) = chain.get_block(&self.block.parent_root())? {
                        block
                    } else {
                        return Ok(None);
                    };

                // Load the parent blocks state from the database, returning an error if it is not found.
                // It is an error because if we know the parent block we should also know the parent state.
                let parent_state_root = parent_block.state_root();
                let parent_state = chain
                    .get_state(&parent_state_root, Some(parent_block.slot()))?
                    .ok_or_else(|| {
                        BeaconChainError::DBInconsistent(format!(
                            "Missing state {:?}",
                            parent_state_root
                        ))
                    })?;

                Ok(Some(BeaconSnapshot {
                    beacon_block: parent_block,
                    beacon_block_root: self.block.parent_root(),
                    beacon_state: parent_state,
                    beacon_state_root: parent_state_root,
                }))
            });

        metrics::stop_timer(db_read_timer);

        result
    }
}
*/

fn load_parent<T: BeaconChainTypes>(
    block: &BeaconBlock<T::EthSpec>,
    chain: &BeaconChain<T>,
) -> Result<BeaconSnapshot<T::EthSpec>, BlockProcessingError> {
    let db_read_timer = metrics::start_timer(&metrics::BLOCK_PROCESSING_DB_READ);

    // Reject any block if its parent is not known to fork choice.
    //
    // A block that is not in fork choice is either:
    //
    //  - Not yet imported: we should reject this block because we should only import a child
    //  after its parent has been fully imported.
    //  - Pre-finalized: if the parent block is _prior_ to finalization, we should ignore it
    //  because it will revert finalization. Note that the finalized block is stored in fork
    //  choice, so we will not reject any child of the finalized block (this is relevant during
    //  genesis).
    if !chain.fork_choice.contains_block(&block.parent_root) {
        return Err(BlockProcessingError::UnknownParent(block.parent_root));
    }

    // Load the parent block and state from disk, returning early if it's not available.
    let result = chain
        .snapshot_cache
        .try_write_for(BLOCK_PROCESSING_CACHE_LOCK_TIMEOUT)
        .and_then(|mut snapshot_cache| snapshot_cache.try_remove(block.parent_root))
        .map(|snapshot| Ok(Some(snapshot)))
        .unwrap_or_else(|| {
            // Load the blocks parent block from the database, returning invalid if that block is not
            // found.
            //
            // We don't return a DBInconsistent error here since it's possible for a block to
            // exist in fork choice but not in the database yet. In such a case we simply
            // indicate that we don't yet know the parent.
            let parent_block = if let Some(block) = chain.get_block(&block.parent_root)? {
                block
            } else {
                return Ok(None);
            };

            // Load the parent blocks state from the database, returning an error if it is not found.
            // It is an error because if we know the parent block we should also know the parent state.
            let parent_state_root = parent_block.state_root();
            let parent_state = chain
                .get_state(&parent_state_root, Some(parent_block.slot()))?
                .ok_or_else(|| {
                    BeaconChainError::DBInconsistent(format!(
                        "Missing state {:?}",
                        parent_state_root
                    ))
                })?;

            Ok(Some(BeaconSnapshot {
                beacon_block: parent_block,
                beacon_block_root: block.parent_root,
                beacon_state: parent_state,
                beacon_state_root: parent_state_root,
            }))
        })
        .map_err(BlockProcessingError::BeaconChainError)?
        .ok_or_else(|| BlockProcessingError::UnknownParent(block.parent_root));

    metrics::stop_timer(db_read_timer);

    result
}

pub fn cheap_state_advance_to_obtain_committees<E: EthSpec>(
    state: &mut BeaconState<E>,
    block_slot: Slot,
    spec: &ChainSpec,
) -> Result<Option<BeaconState<E>>, BlockProcessingError> {
    let block_epoch = block_slot.epoch(E::slots_per_epoch());
    let state_epoch = state.current_epoch();

    if let Ok(relative_epoch) = RelativeEpoch::from_epoch(state_epoch, block_epoch) {
        state
            .build_committee_cache(relative_epoch, spec)
            .map_err(|e| {
                BlockProcessingError::BeaconChainError(BeaconChainError::BeaconStateError(e))
            })?;

        Ok(None)
    } else {
        let mut state = state.clone_with(CloneConfig::none());

        let relative_epoch = loop {
            match RelativeEpoch::from_epoch(state.current_epoch(), block_epoch) {
                Ok(relative_epoch) => break relative_epoch,
                Err(RelativeEpochError::EpochTooLow { .. }) => {
                    // Don't calculate state roots since they aren't required for calculating
                    // shuffling (achieved by providing Hash256::zero()).
                    per_slot_processing(&mut state, Some(Hash256::zero()), spec).map_err(|e| {
                        BlockProcessingError::BeaconChainError(
                            BeaconChainError::SlotProcessingError(e),
                        )
                    })?;
                }
                Err(RelativeEpochError::EpochTooHigh { .. }) => {
                    return Err(BlockProcessingError::BlockIsEarlierThanParent);
                }
            }
        };

        state
            .build_committee_cache(relative_epoch, spec)
            .map_err(|e| {
                BlockProcessingError::BeaconChainError(BeaconChainError::BeaconStateError(e))
            })?;

        Ok(Some(state))
    }
}

pub fn get_validator_pubkey_cache<T: BeaconChainTypes>(
    chain: &BeaconChain<T>,
) -> Result<RwLockReadGuard<ValidatorPubkeyCache>, BlockProcessingError> {
    chain
        .validator_pubkey_cache
        .try_read_for(VALIDATOR_PUBKEY_CACHE_LOCK_TIMEOUT)
        .ok_or_else(|| BeaconChainError::ValidatorPubkeyCacheLockTimeout)
        .map_err(BlockProcessingError::BeaconChainError)
}

/// Produces an _empty_ `BlockSignatureVerifier`.
///
/// The signature verifier is empty because it does not yet have any of this block's signatures
/// added to it. Use `Self::apply_to_signature_verifier` to apply the signatures.
pub fn produce_signature_verifier<'a, T>(
    state: &'a BeaconState<T::EthSpec>,
    validator_pubkey_cache: &'a ValidatorPubkeyCache,
    spec: &'a ChainSpec,
) -> BlockSignatureVerifier<'a, T::EthSpec, impl Fn(usize) -> Option<Cow<'a, G1Point>> + Clone>
where
    T: BeaconChainTypes,
{
    BlockSignatureVerifier::new(
        state,
        move |validator_index| {
            // Disallow access to any validator pubkeys that are not in the current beacon
            // state.
            if validator_index < state.validators.len() {
                validator_pubkey_cache
                    .get(validator_index)
                    .map(|pk| Cow::Borrowed(pk.as_point()))
            } else {
                None
            }
        },
        spec,
    )
}