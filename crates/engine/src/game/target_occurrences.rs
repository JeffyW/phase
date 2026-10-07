//! CR 400.7 + CR 601.2c + CR 608.2b: the single writer for target OCCURRENCES.
//!
//! An occurrence is one position of a `ResolvedAbility`'s `targets` together
//! with the incarnation announced for it (`target_pins`, aligned
//! position-for-position). CR 115.3 lets one object fill several target
//! positions, and CR 400.7 makes an object that left and returned a new object
//! that can share its predecessor's id, so a pin belongs to a POSITION, never
//! to an object id. Every operation here moves targets and pins together, so
//! the two vectors cannot drift apart.
//!
//! Two index domains exist and are never converted implicitly:
//! - a declared address (`RetargetSlotAddress { path, slot }`, or a node-local
//!   declared position) indexes the UNPRUNED announcement on the stack
//!   carrier; the declared view, retarget and the fizzle stamp read through it;
//! - a [`StoredIndex`] indexes the projected execution node that resolution
//!   validation produced, which is what effect handlers read.
//!
//! [`declared_to_stored`] is the only bridge between them.

use crate::types::ability::{ResolvedAbility, TargetRef};
use crate::types::game_state::GameState;
use crate::types::identifiers::ObjectIncarnationRef;

/// A position in a node's projected (post-validation) `targets`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct StoredIndex(pub(crate) usize);

/// CR 608.2b: what resolution validation decided for one announced occurrence
/// of a node. Storage and declared legality are separate questions: an arm may
/// keep an illegal occurrence in storage so later roles keep their positions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum OccurrenceVerdict {
    /// Legal: stored and published.
    Legal,
    /// Illegal, but kept in storage so later roles keep their positions
    /// (multi-role mana, damage-replacement role layouts). Published as a
    /// hole and stamped illegal.
    IllegalRetained,
    /// Illegal and pruned from storage. Published as a hole and stamped
    /// illegal.
    Dropped,
    /// Not validated as a target of this node (an `Attach` node's unclaimed
    /// tail, carried for a downstream sibling): stored unchanged and never
    /// stamped. It keeps its declared position in the view unchanged, so the
    /// view stays index-aligned with `declared_targets_in_chain`.
    PassThrough,
}

impl OccurrenceVerdict {
    /// Whether the occurrence stays in the projected execution node.
    pub(crate) fn is_stored(self) -> bool {
        !matches!(self, Self::Dropped)
    }

    /// CR 608.2b: whether the occurrence is an illegal declared target — a
    /// hole in the declared view and an illegal slot in the fizzle stamp.
    pub(crate) fn is_declared_illegal(self) -> bool {
        matches!(self, Self::IllegalRetained | Self::Dropped)
    }
}

/// Map each declared (unpruned) occurrence of a node to its position in the
/// projected execution node.
///
/// Input domain: `verdicts`, one per DECLARED occurrence of the node (all of
/// its announced `targets`, aligned position-for-position). Output domain:
/// the same length; `Some(StoredIndex(s))` for a stored occurrence (`Legal`,
/// `IllegalRetained` and `PassThrough` all occupy storage, in declared order),
/// `None` for a `Dropped` one. Whether a stored occurrence is also a published
/// declared slot is a separate question (`OccurrenceVerdict::is_declared_illegal`
/// and `chain_node_positions`); this map answers only where it is stored.
pub(crate) fn declared_to_stored(verdicts: &[OccurrenceVerdict]) -> Vec<Option<StoredIndex>> {
    let mut next = 0usize;
    verdicts
        .iter()
        .map(|verdict| {
            verdict.is_stored().then(|| {
                let index = StoredIndex(next);
                next += 1;
                index
            })
        })
        .collect()
}

/// Why a persisted node's occurrence pins were refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TargetPinAlignmentError {
    /// `target_pins` is non-empty and not the same length as `targets`.
    LengthMismatch,
    /// A pin names a different object than the target at its position.
    IdentityMismatch,
    /// A player position carries a pin.
    PlayerPinned,
    /// Both the positional pins and the legacy keyed alias are present.
    AmbiguousLegacyAlias,
}

impl std::fmt::Display for TargetPinAlignmentError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::LengthMismatch => "target pins are not aligned with targets",
            Self::IdentityMismatch => "a target pin names a different object than its position",
            Self::PlayerPinned => "a player target position carries an object pin",
            Self::AmbiguousLegacyAlias => {
                "both positional target pins and the legacy keyed alias are present"
            }
        })
    }
}

/// CR 400.7: check one node's stored occurrence pins (not its continuations).
fn node_pin_alignment(ability: &ResolvedAbility) -> Result<(), TargetPinAlignmentError> {
    if ability.target_pins.is_empty() {
        return Ok(());
    }
    if !ability.legacy_selected_target_incarnations.is_empty() {
        return Err(TargetPinAlignmentError::AmbiguousLegacyAlias);
    }
    if ability.target_pins.len() != ability.targets.len() {
        return Err(TargetPinAlignmentError::LengthMismatch);
    }
    for (target, pin) in ability.targets.iter().zip(&ability.target_pins) {
        match (target, pin) {
            (_, None) => {}
            (TargetRef::Object(id), Some(pin)) if pin.object_id == *id => {}
            (TargetRef::Object(_), Some(_)) => {
                return Err(TargetPinAlignmentError::IdentityMismatch)
            }
            (TargetRef::Player(_), Some(_)) => return Err(TargetPinAlignmentError::PlayerPinned),
        }
    }
    Ok(())
}

/// CR 400.7: validate every node of a chain (sub and else branches included).
/// Release-build check used at the persisted-restore boundary.
pub fn validate_target_pin_alignment(
    ability: &ResolvedAbility,
) -> Result<(), TargetPinAlignmentError> {
    node_pin_alignment(ability)?;
    if let Some(sub) = ability.sub_ability.as_deref() {
        validate_target_pin_alignment(sub)?;
    }
    if let Some(other) = ability.else_ability.as_deref() {
        validate_target_pin_alignment(other)?;
    }
    Ok(())
}

/// Legacy keyed alias: the first pin recorded for `id` (no pin invented).
fn first_matching_pin(
    pins: &[ObjectIncarnationRef],
    id: crate::types::identifiers::ObjectId,
) -> Option<ObjectIncarnationRef> {
    pins.iter().find(|pin| pin.object_id == id).copied()
}

impl ResolvedAbility {
    /// CR 400.7: the pin of every occurrence, aligned with `targets`. A legacy
    /// keyed decode is read as "each object occurrence gets the first legacy
    /// pin with its id; players get none".
    pub fn aligned_target_pins(&self) -> Vec<Option<ObjectIncarnationRef>> {
        (0..self.targets.len())
            .map(|index| self.target_pin_at(index))
            .collect()
    }

    /// CR 400.7: the incarnation announced for the occurrence at `index`, if
    /// any. Positional: two occurrences holding the same id can carry
    /// different pins (a retained old object and an elected new one).
    pub fn target_pin_at(&self, index: usize) -> Option<ObjectIncarnationRef> {
        let TargetRef::Object(id) = self.targets.get(index)? else {
            return None;
        };
        if self.target_pins.is_empty() {
            return first_matching_pin(&self.legacy_selected_target_incarnations, *id);
        }
        debug_assert_eq!(
            node_pin_alignment(self),
            Ok(()),
            "occurrence pins drifted from targets"
        );
        self.target_pins
            .get(index)
            .copied()
            .flatten()
            .filter(|pin| pin.object_id == *id)
    }

    /// CR 400.7 + CR 608.2b: whether the occurrence at `index` may still be
    /// affected — it is a player, it carries no pin, or its pinned incarnation
    /// is still the live object.
    pub fn target_occurrence_is_current(&self, index: usize, state: &GameState) -> bool {
        self.target_pin_at(index)
            .is_none_or(|pin| pin.is_current(state))
    }

    /// CR 115.7 + CR 400.7: whether writing `new` at `index` changes the
    /// occurrence — a different target, or the same id whose announced
    /// incarnation is gone (so `new` names the new object).
    pub fn retarget_requires_pin_refresh_at(
        &self,
        index: usize,
        new: &TargetRef,
        state: &GameState,
    ) -> bool {
        self.targets.get(index) != Some(new)
            || (matches!(new, TargetRef::Object(_))
                && !self.target_occurrence_is_current(index, state))
    }

    /// Normalize the legacy alias into positional form and return the
    /// aligned pins for mutation. Every writer below starts here.
    fn begin_occurrence_write(&mut self) -> Vec<Option<ObjectIncarnationRef>> {
        let pins = self.aligned_target_pins();
        self.legacy_selected_target_incarnations.clear();
        pins
    }

    /// Store `pins` (aligned with the already-written `targets`), compacting
    /// an all-unpinned node to the empty form.
    fn finish_occurrence_write(&mut self, pins: Vec<Option<ObjectIncarnationRef>>) {
        debug_assert_eq!(pins.len(), self.targets.len());
        self.target_pins = if pins.iter().all(Option::is_none) {
            Vec::new()
        } else {
            pins
        };
        debug_assert_eq!(node_pin_alignment(self), Ok(()));
    }

    /// CR 601.2c + CR 400.7: announce — pin every object occurrence to the
    /// live incarnation it names now. Non-recursive; the chain-wide wrapper is
    /// `capture_target_incarnations_recursive`.
    pub fn announce_target_pins(&mut self, state: &GameState) {
        let _ = self.begin_occurrence_write();
        let pins = self
            .targets
            .iter()
            .map(|target| match target {
                TargetRef::Object(id) => {
                    state.objects.get(id).map(ObjectIncarnationRef::from_object)
                }
                TargetRef::Player(_) => None,
            })
            .collect();
        self.finish_occurrence_write(pins);
    }

    /// Replace every occurrence with `targets`, carrying pins forward by the
    /// legacy rule: each new object occurrence takes the pin of the first old
    /// occurrence holding that object (no pin invented). For writers that
    /// rebuild a node from scratch at resolution time; lifecycle writers that
    /// know their source occurrences use the positional operations instead.
    pub fn set_targets(&mut self, targets: Vec<TargetRef>) {
        let old_pins = self.begin_occurrence_write();
        let old_targets = std::mem::replace(&mut self.targets, targets);
        let pins = self
            .targets
            .iter()
            .map(|target| match target {
                TargetRef::Object(_) => old_targets
                    .iter()
                    .position(|old| old == target)
                    .and_then(|found| old_pins[found]),
                TargetRef::Player(_) => None,
            })
            .collect();
        self.finish_occurrence_write(pins);
    }

    /// Replace every occurrence with unpinned `targets`: objects chosen at
    /// resolution, never announced as targets of this node.
    pub fn set_unpinned_targets(&mut self, targets: Vec<TargetRef>) {
        let _ = self.begin_occurrence_write();
        self.targets = targets;
        self.finish_occurrence_write(vec![None; self.targets.len()]);
    }

    /// Copy `source`'s occurrences (targets and pins, position for position).
    pub fn mirror_targets_from(&mut self, source: &ResolvedAbility) {
        let _ = self.begin_occurrence_write();
        self.targets = source.targets.clone();
        self.finish_occurrence_write(source.aligned_target_pins());
    }

    /// Replace every occurrence with explicit `(target, pin)` occurrences.
    pub fn replace_target_occurrences(
        &mut self,
        occurrences: Vec<(TargetRef, Option<ObjectIncarnationRef>)>,
    ) {
        let _ = self.begin_occurrence_write();
        let (targets, pins): (Vec<_>, Vec<_>) = occurrences
            .into_iter()
            .map(|(target, pin)| {
                let pin = match &target {
                    TargetRef::Object(id) => pin.filter(|pin| pin.object_id == *id),
                    TargetRef::Player(_) => None,
                };
                (target, pin)
            })
            .unzip();
        self.targets = targets;
        self.finish_occurrence_write(pins);
    }

    /// The node's occurrences as `(target, pin)` pairs.
    pub fn target_occurrences(&self) -> Vec<(TargetRef, Option<ObjectIncarnationRef>)> {
        self.targets
            .iter()
            .cloned()
            .zip(self.aligned_target_pins())
            .collect()
    }

    /// CR 115.7: write one occurrence — `target` with exactly `pin`. The
    /// caller decides the pin: the live incarnation for a changed occurrence,
    /// the announced one (`target_pin_at`) for an unchanged one.
    pub fn set_target_at(
        &mut self,
        index: usize,
        target: TargetRef,
        pin: Option<ObjectIncarnationRef>,
    ) {
        let mut pins = self.begin_occurrence_write();
        if index >= self.targets.len() {
            self.finish_occurrence_write(pins);
            return;
        }
        let pin = match &target {
            TargetRef::Object(id) => pin.filter(|pin| pin.object_id == *id),
            TargetRef::Player(_) => None,
        };
        self.targets[index] = target;
        pins[index] = pin;
        self.finish_occurrence_write(pins);
    }

    /// Append an unpinned occurrence (announcement-time assignment, before
    /// pins are captured, or a resolution-time choice).
    pub fn push_target(&mut self, target: TargetRef) {
        let mut pins = self.begin_occurrence_write();
        self.targets.push(target);
        pins.push(None);
        self.finish_occurrence_write(pins);
    }

    /// Insert an occurrence at `index` with exactly `pin`.
    pub(crate) fn insert_target(
        &mut self,
        index: usize,
        target: TargetRef,
        pin: Option<ObjectIncarnationRef>,
    ) {
        let mut pins = self.begin_occurrence_write();
        let index = index.min(self.targets.len());
        let pin = match &target {
            TargetRef::Object(id) => pin.filter(|pin| pin.object_id == *id),
            TargetRef::Player(_) => None,
        };
        self.targets.insert(index, target);
        pins.insert(index, pin);
        self.finish_occurrence_write(pins);
    }

    /// Remove every occurrence.
    pub fn clear_targets(&mut self) {
        let _ = self.begin_occurrence_write();
        self.targets.clear();
        self.finish_occurrence_write(Vec::new());
    }

    /// CR 608.2b: storage projection of resolution validation — keep the
    /// occurrences whose verdict is stored (`Legal`, `IllegalRetained`,
    /// `PassThrough`), removing `Dropped` ones from targets and pins
    /// together. `verdicts` is aligned with the node's announced occurrences.
    pub(crate) fn retain_target_occurrences(&mut self, verdicts: &[OccurrenceVerdict]) {
        debug_assert_eq!(verdicts.len(), self.targets.len());
        let positions: Vec<usize> = declared_to_stored(verdicts)
            .into_iter()
            .enumerate()
            .filter_map(|(declared, stored)| stored.map(|_| declared))
            .collect();
        self.project_target_occurrences(&positions);
    }

    /// Keep exactly the occurrences at `positions` (in that order), moving
    /// each pin with its target. An occurrence projection (mana role scoping,
    /// survivor storage), never a re-derivation by object id.
    pub(crate) fn project_target_occurrences(&mut self, positions: &[usize]) {
        let pins = self.begin_occurrence_write();
        let (targets, pins): (Vec<_>, Vec<_>) = positions
            .iter()
            .filter_map(|&index| Some((self.targets.get(index)?.clone(), pins[index])))
            .unzip();
        self.targets = targets;
        self.finish_occurrence_write(pins);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::ability::{Effect, ResolvedAbility};
    use crate::types::identifiers::ObjectId;
    use crate::types::player::PlayerId;

    fn pin(id: u64, incarnation: u64) -> ObjectIncarnationRef {
        ObjectIncarnationRef::of(ObjectId(id), incarnation)
    }

    fn node(targets: Vec<TargetRef>) -> ResolvedAbility {
        ResolvedAbility::new(
            Effect::Unimplemented {
                name: "test".to_string(),
                description: None,
            },
            targets,
            ObjectId(99),
            PlayerId(0),
        )
    }

    /// CR 400.7 + CR 115.3: a legacy keyed decode of `[P1, A, A]` gives each
    /// object occurrence the first legacy pin with its id, the player none,
    /// and invents nothing.
    #[test]
    fn legacy_alias_gives_each_occurrence_the_first_matching_pin() {
        let a = ObjectId(7);
        let mut ability = node(vec![
            TargetRef::Player(PlayerId(1)),
            TargetRef::Object(a),
            TargetRef::Object(a),
        ]);
        ability.legacy_selected_target_incarnations = vec![pin(7, 3), pin(7, 4), pin(9, 1)];
        assert_eq!(
            ability.aligned_target_pins(),
            vec![None, Some(pin(7, 3)), Some(pin(7, 3))]
        );

        // The first write normalizes into positional form and drops the alias.
        ability.set_target_at(2, TargetRef::Object(a), Some(pin(7, 4)));
        assert!(ability.legacy_selected_target_incarnations.is_empty());
        assert_eq!(
            ability.target_pins,
            vec![None, Some(pin(7, 3)), Some(pin(7, 4))]
        );
        assert_eq!(validate_target_pin_alignment(&ability), Ok(()));
    }

    /// CR 400.7: the legacy alias deserializes from the old wire key and is
    /// never re-serialized; the positional form round-trips.
    #[test]
    fn legacy_wire_key_decodes_and_positional_pins_round_trip() {
        let a = ObjectId(7);
        let mut ability = node(vec![TargetRef::Player(PlayerId(1)), TargetRef::Object(a)]);
        ability.announce_target_pins(&GameState::default());
        let mut legacy = serde_json::to_value(&ability).unwrap();
        legacy["selected_target_incarnations"] = serde_json::json!([pin(7, 5)]);
        let decoded: ResolvedAbility = serde_json::from_value(legacy).unwrap();
        assert_eq!(decoded.aligned_target_pins(), vec![None, Some(pin(7, 5))]);

        let mut positional = decoded.clone();
        positional.mirror_targets_from(&decoded);
        let json = serde_json::to_value(&positional).unwrap();
        assert!(json.get("selected_target_incarnations").is_none());
        let back: ResolvedAbility = serde_json::from_value(json).unwrap();
        assert_eq!(back.target_pins, vec![None, Some(pin(7, 5))]);
        assert_eq!(back, decoded, "legacy and positional forms compare equal");
    }

    /// CR 400.7: alignment validation refuses a length drift, a pin naming a
    /// different object than its position, a pinned player, and both forms.
    #[test]
    fn alignment_validation_refuses_mismatched_pins() {
        let mut ability = node(vec![TargetRef::Object(ObjectId(7))]);
        ability.target_pins = vec![Some(pin(8, 1))];
        assert_eq!(
            validate_target_pin_alignment(&ability),
            Err(TargetPinAlignmentError::IdentityMismatch)
        );
        ability.target_pins = vec![Some(pin(7, 1)), None];
        assert_eq!(
            validate_target_pin_alignment(&ability),
            Err(TargetPinAlignmentError::LengthMismatch)
        );
        let mut player = node(vec![TargetRef::Player(PlayerId(0))]);
        player.target_pins = vec![Some(pin(7, 1))];
        assert_eq!(
            validate_target_pin_alignment(&player),
            Err(TargetPinAlignmentError::PlayerPinned)
        );
        let mut both = node(vec![TargetRef::Object(ObjectId(7))]);
        both.target_pins = vec![Some(pin(7, 1))];
        both.legacy_selected_target_incarnations = vec![pin(7, 1)];
        assert_eq!(
            validate_target_pin_alignment(&both),
            Err(TargetPinAlignmentError::AmbiguousLegacyAlias)
        );
        let mut chain = node(vec![TargetRef::Object(ObjectId(7))]);
        chain.target_pins = vec![Some(pin(7, 1))];
        chain.sub_ability = Some(Box::new(ability));
        assert_eq!(
            validate_target_pin_alignment(&chain),
            Err(TargetPinAlignmentError::LengthMismatch),
            "a continuation's misalignment is refused too"
        );
    }

    /// CR 608.2b: the storage projection keeps `Legal`, `IllegalRetained` and
    /// `PassThrough` occurrences with their own pins, removes `Dropped` ones,
    /// and `declared_to_stored` maps each declared position to its projected
    /// index.
    #[test]
    fn retain_moves_pins_with_their_occurrences() {
        let a = ObjectId(7);
        let mut ability = node(vec![
            TargetRef::Object(a),
            TargetRef::Object(a),
            TargetRef::Object(ObjectId(8)),
            TargetRef::Object(ObjectId(9)),
        ]);
        ability.replace_target_occurrences(vec![
            (TargetRef::Object(a), Some(pin(7, 1))),
            (TargetRef::Object(a), Some(pin(7, 2))),
            (TargetRef::Object(ObjectId(8)), Some(pin(8, 1))),
            (TargetRef::Object(ObjectId(9)), None),
        ]);
        let verdicts = [
            OccurrenceVerdict::Dropped,
            OccurrenceVerdict::Legal,
            OccurrenceVerdict::IllegalRetained,
            OccurrenceVerdict::PassThrough,
        ];
        ability.retain_target_occurrences(&verdicts);
        assert_eq!(
            ability.target_occurrences(),
            vec![
                (TargetRef::Object(a), Some(pin(7, 2))),
                (TargetRef::Object(ObjectId(8)), Some(pin(8, 1))),
                (TargetRef::Object(ObjectId(9)), None),
            ],
            "the surviving A keeps ITS pin, not the dropped occurrence's"
        );
        assert_eq!(
            declared_to_stored(&verdicts),
            vec![
                None,
                Some(StoredIndex(0)),
                Some(StoredIndex(1)),
                Some(StoredIndex(2))
            ]
        );
    }
}
