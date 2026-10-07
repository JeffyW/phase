//! CR 707.10c + CR 722.3c + CR 702.192a: the copy target walk
//! (`WaitingFor::CopyRetarget`).
//!
//! One walk shape serves two operations, selected by `CopyChoiceMode`:
//!
//! - **Retarget** (CR 707.10c): a copy that holds targets; its controller may
//!   choose new ones. Positions are the copy's chain-addressed declared targets
//!   (`chain_retarget_slots`). Each pick is `None` (keep) or `Some(t)` (choose),
//!   and the offered picks at every step are exactly those with a legal
//!   completion under the reducer's own validator
//!   (`retarget_completion::RetargetSearch`). Finalization submits the picks to
//!   `engine::validate_retarget_submission`.
//! - **Announce** (CR 722.3c / CR 702.192a): a freshly cast copy announces its
//!   targets (CR 601.2c). Nothing can be kept; the offered targets at every step
//!   come from the production casting walk
//!   (`build_target_selection_progress_for_ability`), and finalization is the
//!   production announcement assignment (`assign_selected_slots_in_chain`,
//!   which captures the announcement pins).
//!
//! The decided prefix is `picks`; `current_slot == picks.len()`. Permissions
//! (`CopyTargetSlot::can_keep`, `can_keep_rest`) and the current slot's
//! `legal_alternatives` are engine-derived at every open, advance and restore.

use crate::game::ability_utils::{
    assign_selected_slots_in_chain, build_target_selection_progress_for_ability, build_target_slots,
};
use crate::game::engine::EngineError;
use crate::game::retarget_completion::{RetargetPick, RetargetSearch};
use crate::types::ability::{EffectKind, ResolvedAbility};
use crate::types::game_state::{CopyChoiceMode, CopyTargetSlot, GameState, WaitingFor};
use crate::types::identifiers::ObjectId;
use crate::types::player::PlayerId;

/// The fixed fields of one walk, carried from prompt to prompt.
#[derive(Debug, Clone)]
pub(crate) struct CopyWalk {
    pub(crate) player: PlayerId,
    pub(crate) copy_id: ObjectId,
    pub(crate) effect_kind: EffectKind,
    pub(crate) effect_source_id: Option<ObjectId>,
    pub(crate) paradigm_remaining_offers: Option<Vec<ObjectId>>,
    pub(crate) mode: CopyChoiceMode,
}

/// What a walk step produced.
#[derive(Debug, Clone)]
pub(crate) enum CopyWalkStep {
    /// The walk continues at this prompt.
    Prompt(Box<WaitingFor>),
    /// Every position is decided; finalize with these picks.
    Complete(Vec<RetargetPick>),
}

fn copy_stack_index(state: &GameState, copy_id: ObjectId) -> Option<usize> {
    state.stack.iter().position(|entry| entry.id == copy_id)
}

fn copy_ability(state: &GameState, copy_id: ObjectId) -> Option<&ResolvedAbility> {
    state
        .stack
        .iter()
        .find(|entry| entry.id == copy_id)
        .and_then(|entry| entry.ability())
}

/// CR 707.10c: the retarget search over the copy's addressed positions.
pub(crate) fn copy_retarget_search(
    state: &GameState,
    copy_id: ObjectId,
) -> Option<RetargetSearch<'_>> {
    RetargetSearch::for_stack_entry(state, copy_stack_index(state, copy_id)?)
}

/// Build the prompt (or completion) for `walk` with decided prefix `picks`.
/// `Ok(None)` for a Retarget walk on a copy with no addressed position.
pub(crate) fn walk_step(
    state: &GameState,
    walk: &CopyWalk,
    picks: Vec<RetargetPick>,
) -> Result<Option<CopyWalkStep>, EngineError> {
    match walk.mode {
        CopyChoiceMode::Retarget => {
            let Some(search) = copy_retarget_search(state, walk.copy_id) else {
                return Ok(None);
            };
            if picks.len() >= search.len() {
                return Ok(Some(CopyWalkStep::Complete(picks)));
            }
            let offered = search.offered(&picks);
            let position = picks.len();
            let target_slots = (0..search.len())
                .map(|i| CopyTargetSlot {
                    current: Some(search.current_targets()[i].clone()),
                    legal_alternatives: if i == position {
                        offered.alternatives.clone()
                    } else {
                        search.slot_pools()[i].clone()
                    },
                    address: Some(search.slots()[i].clone()),
                    can_keep: i == position && offered.can_keep,
                })
                .collect();
            let can_keep_rest = search.can_keep_rest(&picks);
            Ok(Some(CopyWalkStep::Prompt(Box::new(
                WaitingFor::CopyRetarget {
                    player: walk.player,
                    copy_id: walk.copy_id,
                    target_slots,
                    effect_kind: walk.effect_kind,
                    effect_source_id: walk.effect_source_id,
                    current_slot: position,
                    paradigm_remaining_offers: walk.paradigm_remaining_offers.clone(),
                    mode: Some(CopyChoiceMode::Retarget),
                    picks: Some(picks),
                    can_keep_rest,
                },
            ))))
        }
        CopyChoiceMode::Announce => {
            let ability = copy_ability(state, walk.copy_id).ok_or_else(|| {
                EngineError::InvalidAction("Copy is no longer on the stack".to_string())
            })?;
            let slots = build_target_slots(state, ability)?;
            if slots.is_empty() {
                return Ok(Some(CopyWalkStep::Complete(picks)));
            }
            // CR 601.2c: the production casting walk; it validates the prefix,
            // auto-skips optional slots with no legal target, and offers only
            // targets with a legal completion.
            let progress = build_target_selection_progress_for_ability(
                state,
                ability,
                &slots,
                &ability.target_constraints,
                picks.len(),
                picks,
            )?;
            if progress.current_slot >= slots.len() {
                return Ok(Some(CopyWalkStep::Complete(progress.selected_slots)));
            }
            let target_slots = slots
                .iter()
                .enumerate()
                .map(|(i, slot)| CopyTargetSlot {
                    current: progress.selected_slots.get(i).cloned().flatten(),
                    legal_alternatives: if i == progress.current_slot {
                        progress.current_legal_targets.clone()
                    } else {
                        slot.legal_targets.clone()
                    },
                    address: None,
                    can_keep: false,
                })
                .collect();
            Ok(Some(CopyWalkStep::Prompt(Box::new(
                WaitingFor::CopyRetarget {
                    player: walk.player,
                    copy_id: walk.copy_id,
                    target_slots,
                    effect_kind: walk.effect_kind,
                    effect_source_id: walk.effect_source_id,
                    current_slot: progress.current_slot,
                    paradigm_remaining_offers: walk.paradigm_remaining_offers.clone(),
                    mode: Some(CopyChoiceMode::Announce),
                    picks: Some(progress.selected_slots),
                    can_keep_rest: false,
                },
            ))))
        }
    }
}

/// CR 707.10c / CR 601.2c: whether `pick` is an answerable choice at position
/// `picks.len()` of the walk — the reducer gate for `ChooseTarget`.
pub(crate) fn pick_is_admissible(
    state: &GameState,
    walk: &CopyWalk,
    picks: &[RetargetPick],
    pick: &RetargetPick,
) -> Result<bool, EngineError> {
    match walk.mode {
        CopyChoiceMode::Retarget => Ok(copy_retarget_search(state, walk.copy_id)
            .is_some_and(|search| search.admits(picks, pick))),
        CopyChoiceMode::Announce => {
            let ability = copy_ability(state, walk.copy_id).ok_or_else(|| {
                EngineError::InvalidAction("Copy is no longer on the stack".to_string())
            })?;
            let slots = build_target_slots(state, ability)?;
            let progress = build_target_selection_progress_for_ability(
                state,
                ability,
                &slots,
                &ability.target_constraints,
                picks.len(),
                picks.to_vec(),
            )?;
            if progress.current_slot > picks.len() {
                // CR 115.6 / CR 115.10a: the casting walk decides this position
                // itself (an optional slot with no legal target is skipped, a
                // binder is announced); only its own value replays.
                return Ok(progress.selected_slots.get(picks.len()) == Some(pick));
            }
            // CR 601.2c: a fresh announcement has nothing to keep.
            Ok(pick
                .as_ref()
                .is_some_and(|target| progress.current_legal_targets.contains(target)))
        }
    }
}

/// CR 707.10c: whether keeping every remaining position completes — the
/// reducer gate for `KeepAllCopyTargets`. Never for an announcement.
pub(crate) fn keep_rest_is_admissible(
    state: &GameState,
    walk: &CopyWalk,
    picks: &[RetargetPick],
) -> bool {
    walk.mode == CopyChoiceMode::Retarget
        && copy_retarget_search(state, walk.copy_id)
            .is_some_and(|search| search.can_keep_rest(picks))
}

/// The copy's ability as it stands after the walk's final picks, without
/// writing: the retarget validator's result (Retarget), or the production
/// announcement assignment with pin capture (Announce).
pub(crate) fn finalized_copy_ability(
    state: &GameState,
    walk: &CopyWalk,
    mut picks: Vec<RetargetPick>,
) -> Result<ResolvedAbility, EngineError> {
    match walk.mode {
        CopyChoiceMode::Retarget => {
            let search = copy_retarget_search(state, walk.copy_id).ok_or_else(|| {
                EngineError::InvalidAction("Copy has no retargetable position".to_string())
            })?;
            picks.resize(search.len(), None);
            search.validate(&picks)
        }
        CopyChoiceMode::Announce => {
            let mut ability = copy_ability(state, walk.copy_id).cloned().ok_or_else(|| {
                EngineError::InvalidAction("Copy is no longer on the stack".to_string())
            })?;
            assign_selected_slots_in_chain(state, &mut ability, &picks)?;
            Ok(ability)
        }
    }
}

/// A pre-mode (legacy) `CopyRetarget` save: infer the walk mode from the
/// slot shape and rebuild the decided prefix. A fully filled walk is a
/// Retarget (a copy holds every target); a filled prefix with an unchosen
/// suffix is an Announce (a fresh copy announcing). A decided Retarget
/// position is rebuilt as an election of its recorded target (`Some`); the
/// shared changed verdict reads a same-live election as unchanged. An explicit
/// `mode` is never overwritten. Any other shape is structurally malformed.
pub(crate) fn legacy_walk_shape(
    target_slots: &[CopyTargetSlot],
    current_slot: usize,
    mode: Option<CopyChoiceMode>,
) -> Result<(CopyChoiceMode, Vec<RetargetPick>), String> {
    if current_slot > target_slots.len() {
        return Err(format!(
            "copy target walk cursor {current_slot} exceeds its {} slots",
            target_slots.len()
        ));
    }
    let prefix_filled = target_slots[..current_slot]
        .iter()
        .all(|slot| slot.current.is_some());
    let suffix_unchosen = target_slots[current_slot..]
        .iter()
        .all(|slot| slot.current.is_none());
    let all_filled = target_slots.iter().all(|slot| slot.current.is_some());
    let inferred = if all_filled {
        CopyChoiceMode::Retarget
    } else if prefix_filled && suffix_unchosen {
        CopyChoiceMode::Announce
    } else {
        return Err("copy target walk has an inconsistent slot shape".to_string());
    };
    let mode = mode.unwrap_or(inferred);
    if !prefix_filled {
        return Err("copy target walk has an unchosen decided slot".to_string());
    }
    let picks = target_slots[..current_slot]
        .iter()
        .map(|slot| slot.current.clone())
        .collect();
    Ok((mode, picks))
}

/// The walk fields of a `CopyRetarget` prompt, with its mode and picks. A
/// legacy prompt without them is read through `legacy_walk_shape`. `None` for
/// any other prompt or a malformed legacy shape.
pub(crate) fn walk_of(waiting_for: &WaitingFor) -> Option<(CopyWalk, Vec<RetargetPick>)> {
    let WaitingFor::CopyRetarget {
        player,
        copy_id,
        target_slots,
        effect_kind,
        effect_source_id,
        current_slot,
        paradigm_remaining_offers,
        mode,
        picks,
        ..
    } = waiting_for
    else {
        return None;
    };
    let (mode, picks) = match (mode, picks) {
        (Some(mode), Some(picks)) => (*mode, picks.clone()),
        _ => legacy_walk_shape(target_slots, *current_slot, *mode).ok()?,
    };
    Some((
        CopyWalk {
            player: *player,
            copy_id: *copy_id,
            effect_kind: *effect_kind,
            effect_source_id: *effect_source_id,
            paradigm_remaining_offers: paradigm_remaining_offers.clone(),
            mode,
        },
        picks,
    ))
}

/// CR 707.10c: open the "may choose new targets" walk for a copy already on
/// the stack. Returns whether a prompt was armed: `false` when the copy has no
/// addressed position (B5: the chain census, not the root's `targets`, decides).
pub(crate) fn open_copy_retarget_walk(
    state: &mut GameState,
    player: PlayerId,
    copy_id: ObjectId,
    effect_kind: EffectKind,
    effect_source_id: ObjectId,
) -> bool {
    let walk = CopyWalk {
        player,
        copy_id,
        effect_kind,
        effect_source_id: Some(effect_source_id),
        paradigm_remaining_offers: None,
        mode: CopyChoiceMode::Retarget,
    };
    // W1: the seed (keep everything) must be a confirmed validator result.
    let seed_confirmed =
        copy_retarget_search(state, copy_id).is_some_and(|search| search.confirm_seed().is_ok());
    if !seed_confirmed {
        return false;
    }
    match walk_step(state, &walk, Vec::new()) {
        Ok(Some(CopyWalkStep::Prompt(prompt))) => {
            state.waiting_for = *prompt;
            true
        }
        _ => false,
    }
}

/// CR 707.10c / CR 601.2c: rebuild a restored `CopyRetarget` walk before the
/// state is published. A pre-mode save's mode and decided prefix are inferred
/// (`legacy_walk_shape`); an explicit mode is kept. The decided picks are
/// replayed through the reducer's own gate (`pick_is_admissible`) and the walk
/// is truncated at the first pick the current board refuses, so the player is
/// re-asked from there (CR 115.7d: an unfinished choice is still the
/// player's). The prompt — the current slot's offered targets, its keep
/// permission, `can_keep_rest`, and the reset suffix — is then re-derived.
/// A fresh announcement with no legal announcement at all is not resumable:
/// restore fails closed rather than publish an unanswerable prompt.
pub(crate) fn restore_copy_target_walk(
    state: &mut GameState,
) -> Result<(), crate::types::game_state::PersistedRestoreError> {
    use crate::types::game_state::PersistedRestoreError;
    let WaitingFor::CopyRetarget {
        player,
        copy_id,
        target_slots,
        effect_kind,
        effect_source_id,
        current_slot,
        paradigm_remaining_offers,
        mode,
        picks,
        ..
    } = &state.waiting_for
    else {
        return Ok(());
    };
    let malformed = |reason: String| PersistedRestoreError::InvalidCopyTargetWalk(reason);
    let (mode, picks) = match (mode, picks) {
        (Some(mode), Some(picks)) => (*mode, picks.clone()),
        (mode, None) => legacy_walk_shape(target_slots, *current_slot, *mode).map_err(malformed)?,
        (None, Some(_)) => {
            return Err(malformed(
                "copy target walk records picks without a mode".to_string(),
            ))
        }
    };
    if picks.len() != *current_slot {
        return Err(malformed(format!(
            "copy target walk cursor {current_slot} does not match its {} decided picks",
            picks.len()
        )));
    }
    let walk = CopyWalk {
        player: *player,
        copy_id: *copy_id,
        effect_kind: *effect_kind,
        effect_source_id: *effect_source_id,
        paradigm_remaining_offers: paradigm_remaining_offers.clone(),
        mode,
    };
    if copy_ability(state, walk.copy_id).is_none() {
        return Err(malformed(format!(
            "copy target walk names {:?}, which is not a copy on the stack",
            walk.copy_id
        )));
    }
    let mut accepted: Vec<RetargetPick> = Vec::with_capacity(picks.len());
    for pick in picks {
        if !pick_is_admissible(state, &walk, &accepted, &pick).unwrap_or(false) {
            break;
        }
        accepted.push(pick);
    }
    match walk_step(state, &walk, accepted.clone()) {
        Ok(Some(CopyWalkStep::Prompt(prompt))) => {
            state.waiting_for = *prompt;
            Ok(())
        }
        // CR 601.2c: no legal announcement exists for a fresh copy.
        Err(_) if mode == CopyChoiceMode::Announce && accepted.is_empty() => {
            Err(PersistedRestoreError::NonResumableCopyAnnouncement {
                copy_id: walk.copy_id,
            })
        }
        Ok(Some(CopyWalkStep::Complete(_))) => Err(malformed(
            "copy target walk has no undecided position".to_string(),
        )),
        Ok(None) => Err(malformed(
            "copy target walk's copy has no retargetable position".to_string(),
        )),
        Err(error) => Err(malformed(format!(
            "copy target walk cannot resume: {error:?}"
        ))),
    }
}
