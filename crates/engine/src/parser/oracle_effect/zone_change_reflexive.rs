//! CR 603.12: support boundary for the "When <zone change> this way, <body>"
//! reflexive triggered ability.
//!
//! `conditions::strip_if_you_do_conditional` lowers that head to the
//! `WhenYouDo` creation gate guarded by `ZoneChangedThisWay`; the runtime then
//! creates the reflexive after the zone change and chooses its targets as it is
//! put on the stack (CR 603.3d). That gate reads the zone-change ledger the
//! immediately preceding instruction published, and the reflexive's body reads
//! only referents the stack object carries. This module decides, per clause,
//! whether both hold; anything else fails closed.

use nom::branch::alt;
use nom::bytes::complete::{tag, take_until};
use nom::combinator::value;
use nom::sequence::preceded;
use nom::Parser;

use super::super::oracle_nom::error::OracleError;
use super::super::oracle_nom::primitives as nom_primitives;
use super::{has_typed_target_widened, ClauseDisposition, ClauseIr};
use crate::types::ability::{
    AbilityCondition, Effect, FilterProp, ObjectScope, QuantityExpr, QuantityRef, TargetFilter,
};

/// The zone-change event a "When … this way" head names.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ThisWayEvent {
    /// CR 701.9a
    Discard,
    /// CR 701.21a
    Sacrifice,
    /// CR 701.13a
    Exile,
    /// CR 701.17a
    Mill,
    /// "put … onto the battlefield" / "enters"
    EnterBattlefield,
    /// "put … into your graveyard" (surveil)
    PutIntoGraveyard,
}

fn parse_this_way_event(input: &str) -> nom::IResult<&str, ThisWayEvent, OracleError<'_>> {
    alt((
        value(ThisWayEvent::Discard, tag("discard")),
        value(ThisWayEvent::Sacrifice, tag("sacrific")),
        value(ThisWayEvent::Exile, tag("exile")),
        value(ThisWayEvent::Mill, tag("mill")),
        value(ThisWayEvent::EnterBattlefield, tag("onto the battlefield")),
        value(ThisWayEvent::EnterBattlefield, tag("enter")),
        value(ThisWayEvent::PutIntoGraveyard, tag("into your graveyard")),
    ))
    .parse(input)
}

/// The event of a lowercase "when <head> this way, …" chunk: the verb right
/// after "you " for the active voice ("when you discard a card this way"),
/// otherwise the last event word before "this way" for the passive voice
/// ("when a creature card is exiled this way").
fn this_way_event(lower: &str) -> Option<ThisWayEvent> {
    let (_, head) = preceded(
        tag::<_, _, OracleError<'_>>("when "),
        take_until(" this way"),
    )
    .parse(lower)
    .ok()?;
    if let Ok((_, event)) = preceded(tag("you "), parse_this_way_event).parse(head) {
        return Some(event);
    }
    let mut last = None;
    let mut remaining = head;
    while let Some(event) = nom_primitives::scan_at_word_boundaries(remaining, |input| {
        parse_this_way_event(input).map(|(rest, event)| (rest, (event, rest)))
    }) {
        last = Some(event.0);
        remaining = event.1;
    }
    last
}

/// CR 608.2c: whether `effect` is an instruction that publishes the `event`'s
/// zone change. Positive allowlist: an unlisted producer fails closed.
fn publishes_event(effect: &Effect, event: ThisWayEvent) -> bool {
    match event {
        ThisWayEvent::Discard => {
            matches!(effect, Effect::Discard { .. } | Effect::DiscardCard { .. })
        }
        ThisWayEvent::Sacrifice => matches!(effect, Effect::Sacrifice { .. }),
        ThisWayEvent::Exile => matches!(
            effect,
            Effect::ExileTop { .. }
                | Effect::ExileFromTopUntil { .. }
                | Effect::ChangeZone { .. }
                | Effect::ChangeZoneAll { .. }
        ),
        ThisWayEvent::Mill => matches!(effect, Effect::Mill { .. }),
        ThisWayEvent::EnterBattlefield => matches!(
            effect,
            Effect::ChangeZone { .. }
                | Effect::ChangeZoneAll { .. }
                | Effect::Dig { .. }
                | Effect::DraftFromSpellbook { .. }
                | Effect::Conjure { .. }
        ),
        ThisWayEvent::PutIntoGraveyard => matches!(effect, Effect::Surveil { .. }),
    }
}

/// CR 608.2c: a play permission over the moved cards ("You may play that card
/// this turn") is not the moving instruction; the runtime leaves the ledger in
/// place across it (`effect_only_grants_play_permission`, the same single
/// effect kind).
fn grants_play_permission(effect: &Effect) -> bool {
    matches!(effect, Effect::GrantCastingPermission { .. })
}

/// CR 603.12 + CR 608.2c: why a clause gated by a "When <zone change> this
/// way" reflexive head cannot be supported, or `None` when it can.
///
/// * `zone_change_reflexive_event_source` — the instruction whose ledger the
///   gate reads (the nearest preceding instruction, skipping a play
///   permission) is not one that performs the head's zone change: a later
///   moving instruction ("Discard a card, then draw a card") or a structural
///   mismatch ("sacrifice … unless you discard", a delayed sacrifice) would
///   make the gate read the wrong cards.
/// * `zone_change_reflexive_unbound_referent` — the body names an object the
///   reflexive stack object cannot carry: a bare `ParentTarget` operand (its
///   target, or a fight's subject) while no earlier instruction announced a
///   target for it to inherit or while the body itself says "target"; a copy
///   with no copy source; or a target filter compared against the ability's
///   own target while that target is being chosen.
pub(super) fn unsupported_zone_change_reflexive(
    condition: Option<&AbilityCondition>,
    effect: &Effect,
    chunk_lower: &str,
    body_lower: &str,
    clauses: &[ClauseIr],
) -> Option<&'static str> {
    let condition = condition?;
    if !condition.has_when_you_do_marker() {
        return None;
    }
    condition.zone_changed_this_way_head()?;
    let producer = clauses
        .iter()
        .rev()
        .filter(|clause| !matches!(clause.disposition, ClauseDisposition::Continue { .. }))
        .find(|clause| !grants_play_permission(&clause.parsed.effect));
    let event_published = match (producer, this_way_event(chunk_lower)) {
        (Some(producer), Some(event)) => publishes_event(&producer.parsed.effect, event),
        _ => false,
    };
    if !event_published {
        return Some("zone_change_reflexive_event_source");
    }
    let body_names_a_target = nom_primitives::scan_at_word_boundaries(body_lower, |input| {
        tag::<_, _, OracleError<'_>>("target ").parse(input)
    })
    .is_some();
    let parent_announced_a_target =
        producer.is_some_and(|producer| has_typed_target_widened(&producer.parsed.effect));
    // The body's object operands: its target, plus the fighting creature of a
    // fight ("it fights ~").
    let fight_subject = match effect {
        Effect::Fight { subject, .. } => Some(subject),
        _ => None,
    };
    let reads_parent_target = effect
        .target_filter()
        .into_iter()
        .chain(fight_subject)
        .any(|operand| matches!(operand, TargetFilter::ParentTarget));
    if reads_parent_target && (body_names_a_target || !parent_announced_a_target) {
        return Some("zone_change_reflexive_unbound_referent");
    }
    // A copy whose source lost its referent ("a token that's a copy of that
    // Aura" parsed with no source), or a target filter that compares a
    // candidate against the ability's own target while that target is being
    // chosen ("with equal or lesser mana value than that spell" read as the
    // target), names no object the stack object can carry.
    let copies_nothing = matches!(
        effect,
        Effect::CopyTokenOf {
            target: TargetFilter::None,
            ..
        }
    );
    let filter_reads_own_target = effect
        .target_filter()
        .is_some_and(filter_compares_against_own_target);
    if copies_nothing || filter_reads_own_target {
        return Some("zone_change_reflexive_unbound_referent");
    }
    None
}

/// Whether a mana-value comparison inside `filter` reads `ObjectScope::Target`
/// — the ability's own target, which does not exist while that target is the
/// one being chosen.
fn filter_compares_against_own_target(filter: &TargetFilter) -> bool {
    match filter {
        TargetFilter::Typed(typed) => typed.properties.iter().any(|property| {
            matches!(
                property,
                FilterProp::Cmc {
                    value: QuantityExpr::Ref {
                        qty: QuantityRef::ObjectManaValue {
                            scope: ObjectScope::Target,
                        },
                    },
                    ..
                }
            )
        }),
        TargetFilter::Or { filters } | TargetFilter::And { filters } => {
            filters.iter().any(filter_compares_against_own_target)
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::{this_way_event, ThisWayEvent};

    #[test]
    fn classifies_active_and_passive_heads() {
        let cases = [
            (
                "when you discard a nonland card this way, x",
                ThisWayEvent::Discard,
            ),
            ("when you exile a card this way, x", ThisWayEvent::Exile),
            (
                "when you sacrifice one or more zombies this way, x",
                ThisWayEvent::Sacrifice,
            ),
            (
                "when you put one or more equipment onto the battlefield this way, x",
                ThisWayEvent::EnterBattlefield,
            ),
            (
                "when a creature card is exiled this way, x",
                ThisWayEvent::Exile,
            ),
            (
                "when one or more cards are milled this way, x",
                ThisWayEvent::Mill,
            ),
            (
                "when an equipment enters this way, x",
                ThisWayEvent::EnterBattlefield,
            ),
            (
                "when a creature is put onto the battlefield this way, x",
                ThisWayEvent::EnterBattlefield,
            ),
        ];
        for (text, expected) in cases {
            assert_eq!(this_way_event(text), Some(expected), "{text}");
        }
        assert_eq!(this_way_event("when you do, x"), None);
    }
}
