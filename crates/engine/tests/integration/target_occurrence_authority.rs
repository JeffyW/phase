//! CR 400.7 + CR 608.2b + CR 701.14b: the positional target-occurrence
//! authority through production casts, persistence and resolution.
//!
//! - **Persistence (R9-1):** a save carrying only the pre-positional keyed pins
//!   restores, re-saves and restores again with no target write in between,
//!   and still names the announced object (stack and resolution-session fence).
//! - **Implicit fight (R9-3):** a Fight node's own declared Elf occurrence is
//!   judged by its own address; another node's occurrence of the same returned
//!   object never stands in for it, and an illegal declared fighter means no
//!   fight rather than a fall-through to another instruction's targets.
//! - **Hand-off (R9-4):** a continuation parked by a Scry pause inherits the
//!   validated parent's occurrences, pins included.
//!
//! Oracle text (Scryfall-verified): Ghostly Flicker, Unsummon, Twincast,
//! Redirect. The Exchange + Fight and Exchange + Scry spells are engine-defined
//! compositions of supported parsed definitions, not fabricated card text.

use engine::game::scenario::{GameRunner, GameScenario, P0, P1};
use engine::parser::oracle::parse_oracle_text;
use engine::types::ability::{
    AbilityDefinition, AbilityKind, Effect, EffectScope, ResolvedAbility, TapStateChange,
    TargetFilter, TargetRef, TypedFilter,
};
use engine::types::actions::{GameAction, ResolveAllScope};
use engine::types::game_state::{PersistedGameState, PersistedRestoreFinalization, WaitingFor};
use engine::types::identifiers::ObjectId;
use engine::types::mana::ManaCost;
use engine::types::phase::Phase;
use engine::types::player::PlayerId;
use engine::types::zones::Zone;

const EXCHANGE: &str = "Exchange control of target artifact and target creature.";
const FLICKER: &str = "Exile two target artifacts, creatures, and/or lands you control, then return those cards to the battlefield under your control.";
const UNSUMMON: &str = "Return target creature to its owner's hand.";
const REDIRECT: &str = "You may choose new targets for target spell.";
const TWINCAST: &str =
    "Copy target instant or sorcery spell. You may choose new targets for the copy.";

fn free_spell(s: &mut GameScenario, p: PlayerId, name: &str, text: &str) -> ObjectId {
    s.add_spell_to_hand_from_oracle(p, name, true, text)
        .with_mana_cost(ManaCost::zero())
        .id()
}

fn ability(r: &GameRunner, source: ObjectId) -> &ResolvedAbility {
    r.state()
        .stack
        .iter()
        .find(|e| e.source_id == source)
        .and_then(|e| e.ability())
        .expect("spell on the stack")
}

fn incarnations(a: &ResolvedAbility) -> Vec<Option<u64>> {
    a.aligned_target_pins()
        .into_iter()
        .map(|p| p.map(|p| p.incarnation))
        .collect()
}

fn priority(r: &mut GameRunner, player: PlayerId) {
    for _ in 0..8 {
        if matches!(r.state().waiting_for, WaitingFor::Priority { player: p } if p == player) {
            return;
        }
        r.act(GameAction::PassPriority).expect("pass priority");
    }
    panic!("priority never reached {player:?}");
}

fn resolve_one(r: &mut GameRunner, source: ObjectId) {
    for _ in 0..32 {
        if !r.state().stack.iter().any(|e| e.source_id == source) {
            return;
        }
        r.act(GameAction::PassPriority).expect("resolve");
    }
    panic!("resolution loop");
}

fn open_retarget_prompt(r: &mut GameRunner) {
    for _ in 0..24 {
        match r.state().waiting_for {
            WaitingFor::RetargetChoice { .. } | WaitingFor::CopyRetarget { .. } => return,
            WaitingFor::OptionalEffectChoice { .. } => {
                r.act(GameAction::DecideOptionalEffect { accept: true })
                    .expect("accept the optional retarget");
            }
            _ => {
                r.act(GameAction::PassPriority)
                    .expect("resolve to the prompt");
            }
        }
    }
    panic!("no retarget prompt");
}

/// Rewrite every positional pin vector in a persisted wire into the
/// pre-positional keyed form the base writer emitted.
fn to_legacy_pin_wire(value: &mut serde_json::Value) -> usize {
    match value {
        serde_json::Value::Object(map) => {
            let mut converted = 0;
            if let Some(pins) = map.remove("target_pins") {
                let keyed: Vec<_> = pins
                    .as_array()
                    .expect("pin array")
                    .iter()
                    .filter(|pin| !pin.is_null())
                    .cloned()
                    .collect();
                map.insert(
                    "selected_target_incarnations".into(),
                    serde_json::json!(keyed),
                );
                converted += 1;
            }
            converted + map.values_mut().map(to_legacy_pin_wire).sum::<usize>()
        }
        serde_json::Value::Array(values) => values.iter_mut().map(to_legacy_pin_wire).sum(),
        _ => 0,
    }
}

fn restore(value: serde_json::Value) -> GameRunner {
    let state = serde_json::from_value::<PersistedGameState>(value)
        .expect("persisted state decodes")
        .prepare_for_restore(PersistedRestoreFinalization::DeferUntilRehydrated)
        .expect("persisted state is admissible")
        .finalize_after_rehydration(|_| Ok(()))
        .expect("restored state is publishable");
    GameRunner::from_state(state)
}

fn save(r: &GameRunner) -> serde_json::Value {
    serde_json::to_value(PersistedGameState::capture(r.state().clone())).unwrap()
}

/// R9-1: Unsummon targets A; Ghostly Flicker blinks A (a new object). The
/// save is rewritten to the legacy keyed pin encoding (optionally with a
/// Resolve All session fence), restored, re-saved WITHOUT any target write,
/// and restored again. Unsummon's target is still the departed A, so the
/// returned A stays on the battlefield (CR 400.7 + CR 608.2b).
fn resave_board(legacy: bool, fence: bool) {
    let mut s = GameScenario::new();
    s.at_phase(Phase::PreCombatMain);
    let a = s.add_creature(P1, "Legacy Target A", 2, 7).id();
    let land = s.add_land_from_oracle(P1, "Blink Land", "").id();
    let spell = free_spell(&mut s, P0, "Unsummon", UNSUMMON);
    let flicker = free_spell(&mut s, P1, "Ghostly Flicker", FLICKER);
    // A held response keeps P1 from auto-passing, so a Resolve All session
    // stays open at its fence.
    let _response = free_spell(&mut s, P1, "Opponent's Unsummon", UNSUMMON);
    let mut r = s.build();
    r.cast(spell).target_object(a).commit();
    assert_eq!(incarnations(ability(&r, spell)), vec![Some(0)], "reach");
    priority(&mut r, P1);
    r.cast(flicker).target_objects(&[a, land]).commit();
    resolve_one(&mut r, flicker);
    assert_ne!(r.state().objects[&a].incarnation, 0, "reach: A blinked");
    if fence {
        priority(&mut r, P0);
        r.act(GameAction::BeginResolveAll {
            max_resolutions: 1,
            scope: ResolveAllScope::Own,
        })
        .expect("production resolution session");
        assert!(
            r.state().stack_resolution_session.is_some(),
            "reach: fence captured"
        );
    }
    let mut wire = save(&r);
    if legacy {
        let owners = to_legacy_pin_wire(&mut wire);
        assert!(
            owners >= if fence { 3 } else { 1 },
            "reach: {owners} legacy owners"
        );
    }
    let first = restore(wire);
    assert_eq!(incarnations(ability(&first, spell)), vec![Some(0)]);
    let resaved = save(&first);
    let resaved_text = resaved.to_string();
    assert!(
        !resaved_text.contains("selected_target_incarnations"),
        "the re-save is positional"
    );
    let mut second = restore(resaved);
    assert_eq!(
        incarnations(ability(&second, spell)),
        vec![Some(0)],
        "the pin survives restore -> save -> restore with no target write"
    );
    if fence {
        let session = second
            .state()
            .stack_resolution_session
            .as_ref()
            .expect("the fence survives");
        assert_eq!(
            session.entries[0]
                .target_pins
                .iter()
                .map(|pin| pin.map(|pin| pin.incarnation))
                .collect::<Vec<_>>(),
            vec![Some(0)],
            "the fence keeps its pin"
        );
    }
    resolve_one(&mut second, spell);
    assert_eq!(
        second.state().objects[&a].zone,
        Zone::Battlefield,
        "the returned A is a new object Unsummon never targeted"
    );
}

#[test]
fn legacy_pins_survive_resave_on_the_stack() {
    resave_board(true, false);
}

#[test]
fn legacy_pins_survive_resave_through_a_resolution_fence() {
    resave_board(true, true);
}

#[test]
fn positional_pins_survive_resave_control() {
    resave_board(false, false);
}

/// R9-3: `[C, B]` exchange root + `Fight { ParentTarget, Elf }` announcing Elf
/// A. Optionally blink A, then Twincast the spell and change only the copy's
/// root position 1 (B -> the returned A) or nothing. Returns the damage marked
/// on `[C, B, A]` by the copy's resolution.
fn fight_board(blink: bool, change_root: bool) -> ([u32; 3], [u32; 3]) {
    let mut s = GameScenario::new();
    s.at_phase(Phase::PreCombatMain);
    let c = s
        .add_creature(P1, "Non-Elf Artifact Creature C", 2, 9)
        .as_artifact()
        .as_creature()
        .id();
    let b = s.add_creature(P1, "Non-Elf Creature B", 3, 9).id();
    let a = s
        .add_creature(P1, "Elf A", 4, 9)
        .with_subtypes(vec!["Elf"])
        .id();
    let land = s.add_land_from_oracle(P1, "Elf Blink Land", "").id();
    let types = vec!["Instant".to_string()];
    let mut root = parse_oracle_text(EXCHANGE, "Fight Probe", &[], &types, &[])
        .abilities
        .remove(0);
    root.sub_ability = Some(Box::new(AbilityDefinition::new(
        AbilityKind::Spell,
        Effect::Fight {
            subject: TargetFilter::ParentTarget,
            target: TargetFilter::Typed(TypedFilter::creature().subtype("Elf".into())),
        },
    )));
    let spell = s
        .add_spell_to_hand(P0, "Fight Probe", true)
        .with_mana_cost(ManaCost::zero())
        .with_ability_definition(root)
        .id();
    let flicker = free_spell(&mut s, P1, "Ghostly Flicker", FLICKER);
    let twincast = free_spell(&mut s, P0, "Twincast", TWINCAST);
    let mut r = s.build();
    r.cast(spell).target_objects(&[c, b, a]).commit();
    let original = ability(&r, spell);
    assert_eq!(
        original.targets,
        vec![TargetRef::Object(c), TargetRef::Object(b)],
        "reach: the exchange owns C, B"
    );
    assert_eq!(
        original.sub_ability.as_ref().unwrap().targets,
        vec![TargetRef::Object(a)],
        "reach: the fight owns its Elf A occurrence"
    );
    if blink {
        priority(&mut r, P1);
        r.cast(flicker).target_objects(&[a, land]).commit();
        resolve_one(&mut r, flicker);
        assert_ne!(r.state().objects[&a].incarnation, 0, "reach: A blinked");
    }
    priority(&mut r, P0);
    r.cast(twincast).target_object(spell).commit();
    open_retarget_prompt(&mut r);
    let WaitingFor::CopyRetarget {
        copy_id,
        target_slots,
        ..
    } = &r.state().waiting_for
    else {
        panic!("expected CopyRetarget");
    };
    let copy_id = *copy_id;
    assert_eq!(target_slots.len(), 2, "reach: the copy addresses C, B");
    r.act(GameAction::ChooseTarget { target: None })
        .expect("keep C");
    r.act(GameAction::ChooseTarget {
        target: change_root.then_some(TargetRef::Object(a)),
    })
    .expect("position 1");
    let copied = r
        .state()
        .stack
        .iter()
        .find(|e| e.id == copy_id)
        .and_then(|e| e.ability())
        .expect("the copy is on the stack")
        .clone();
    if blink && change_root {
        assert_eq!(
            copied.target_pin_at(1).unwrap().incarnation,
            r.state().objects[&a].incarnation,
            "reach: the copy's root names the returned A"
        );
        assert_eq!(
            copied
                .sub_ability
                .as_ref()
                .unwrap()
                .target_pin_at(0)
                .unwrap()
                .incarnation,
            0,
            "reach: the copy's fight still names the departed A"
        );
    }
    let before = [c, b, a].map(|o| r.state().objects[&o].damage_marked);
    for _ in 0..16 {
        if !r.state().stack.iter().any(|e| e.id == copy_id) {
            break;
        }
        r.act(GameAction::PassPriority).expect("resolve the copy");
    }
    let after = [c, b, a].map(|o| r.state().objects[&o].damage_marked);
    (before, after)
}

/// CR 701.14b + CR 608.2b + CR 707.10c: the copy's root now names the
/// returned A, but its fight's own Elf target is the departed A — an illegal
/// instructed fighter, so nothing fights. The root's occurrence of the same
/// object never stands in for the fight's.
#[test]
fn fight_ignores_another_nodes_occurrence_of_the_returned_object() {
    let (before, after) = fight_board(true, true);
    assert_eq!(after, before, "no fight with the stale Elf A");
}

/// CR 701.14b: root unchanged, the fight's Elf A departed — no fight, and the
/// root's two exchange targets are not read as the fighters.
#[test]
fn illegal_declared_fighter_does_not_fall_back_to_the_roots_targets() {
    let (before, after) = fight_board(true, false);
    assert_eq!(after, before, "C and B must not fight");
}

/// Control: no blink — the copy's ally C fights the Elf A normally.
#[test]
fn legal_declared_fighter_fights_control() {
    let (before, after) = fight_board(false, false);
    assert_ne!(after, before, "the all-legal copy fights");
}

/// R9-4: `[A, A]` exchange root with A's artifact type removed after a blink,
/// so Redirect can keep the departed A at the artifact position and elect the
/// returned A at the creature position (mixed pins `[old, new]`). The spell
/// continues with Scry 1, which pauses, then taps "that creature". The
/// continuation parked by the Scry pause must carry the validated root's
/// surviving occurrence WITH its own pin (the returned A), never the departed
/// A's pin and never an unpinned copy.
#[test]
fn scry_pause_continuation_inherits_the_validated_occurrence() {
    let mut s = GameScenario::new();
    s.at_phase(Phase::PreCombatMain);
    let a = s
        .add_creature(P1, "Artifact Creature A", 2, 7)
        .as_artifact()
        .as_creature()
        .id();
    let land = s.add_land_from_oracle(P1, "Blink Land", "").id();
    let types = vec!["Instant".to_string()];
    let mut root = parse_oracle_text(EXCHANGE, "Scry Probe", &[], &types, &[])
        .abilities
        .remove(0);
    let mut scry = parse_oracle_text("Scry 1.", "Scry Probe", &[], &types, &[])
        .abilities
        .remove(0);
    scry.sub_ability = Some(Box::new(AbilityDefinition::new(
        AbilityKind::Spell,
        Effect::SetTapState {
            target: TargetFilter::ParentTarget,
            scope: EffectScope::Single,
            state: TapStateChange::Tap,
        },
    )));
    root.sub_ability = Some(Box::new(scry));
    let spell = s
        .add_spell_to_hand(P0, "Scry Probe", true)
        .with_mana_cost(ManaCost::zero())
        .with_ability_definition(root)
        .id();
    let flicker = free_spell(&mut s, P1, "Ghostly Flicker", FLICKER);
    let redirect = free_spell(&mut s, P1, "Redirect", REDIRECT);
    s.with_library_top(P0, &["Library Card 1", "Library Card 2"]);
    let mut r = s.build();
    r.cast(spell).target_objects(&[a, a]).commit();
    priority(&mut r, P1);
    r.cast(flicker).target_objects(&[a, land]).commit();
    resolve_one(&mut r, flicker);
    let returned = r.state().objects[&a].incarnation;
    assert_ne!(returned, 0, "reach: A blinked");
    r.state_mut().add_transient_continuous_effect(
        a,
        P1,
        engine::types::ability::Duration::UntilEndOfTurn,
        TargetFilter::SpecificObject { id: a },
        vec![engine::types::ability::ContinuousModification::RemoveType {
            core_type: engine::types::CoreType::Artifact,
        }],
        None,
    );
    engine::game::layers::evaluate_layers(r.state_mut());
    priority(&mut r, P1);
    r.cast(redirect).target_object(spell).commit();
    open_retarget_prompt(&mut r);
    r.act(GameAction::RetargetSpell {
        new_targets: vec![TargetRef::Object(a), TargetRef::Object(a)],
    })
    .expect("keep the departed A, elect the returned A");
    assert_eq!(
        incarnations(ability(&r, spell)),
        vec![Some(0), Some(returned)],
        "reach: mixed occurrence pins"
    );
    resolve_one(&mut r, redirect);
    for _ in 0..16 {
        if matches!(r.state().waiting_for, WaitingFor::ScryChoice { .. }) {
            break;
        }
        r.act(GameAction::PassPriority)
            .expect("resolve to the Scry pause");
    }
    assert!(
        matches!(r.state().waiting_for, WaitingFor::ScryChoice { .. }),
        "reach: the Scry pauses, got {:?}",
        r.state().waiting_for
    );
    let parked = &r
        .state()
        .active_ability_continuation()
        .expect("reach: the tap continuation is parked")
        .chain;
    let occurrences: Vec<_> = parked
        .target_occurrences()
        .into_iter()
        .filter(|(target, _)| *target == TargetRef::Object(a))
        .map(|(_, pin)| pin.map(|pin| pin.incarnation))
        .collect();
    assert!(!occurrences.is_empty(), "reach: the continuation names A");
    assert!(
        occurrences.iter().all(|pin| *pin == Some(returned)),
        "the continuation carries the validated occurrence's own pin: {occurrences:?}"
    );
}
