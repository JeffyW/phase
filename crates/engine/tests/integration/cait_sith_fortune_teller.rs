//! Cait Sith, Fortune Teller (#9762): "When you exile a card this way, target
//! creature you control gets +X/+0 until end of turn, where X is that card's
//! mana value." is a reflexive triggered ability (CR 603.12). It is created
//! after the exile, its target is chosen as it is put on the stack (CR 603.3d),
//! and the zone change that triggered it is not re-checked as it resolves
//! (it is not an intervening "if", CR 603.4).
//!
//! Every test drives the printed combat trigger through production priority.

use engine::game::scenario::{GameRunner, GameScenario, P0};
use engine::game::zones::move_to_zone;
use engine::parser::oracle::parse_oracle_text;
use engine::types::ability::{Effect, EffectKind, TargetRef};
use engine::types::actions::GameAction;
use engine::types::events::GameEvent;
use engine::types::game_state::{PersistedGameState, PersistedRestoreFinalization, WaitingFor};
use engine::types::identifiers::ObjectId;
use engine::types::mana::ManaCost;
use engine::types::phase::Phase;
use engine::types::zones::Zone;

const CAIT_SITH: &str = "Lucky Slots — At the beginning of combat on your turn, scry 1, then exile the top card of your library. You may play that card this turn. When you exile a card this way, target creature you control gets +X/+0 until end of turn, where X is that card's mana value.";

fn zone(r: &GameRunner, id: ObjectId) -> Zone {
    r.state().objects[&id].zone
}

struct Fixture {
    runner: GameRunner,
    cait: ObjectId,
    bear: ObjectId,
    top: Option<ObjectId>,
}

/// Cait Sith and a Bear for P0 (two legal targets, so the prompt is
/// observable); the top of P0's library is a mana value 3 card unless
/// `empty_library`.
fn fixture(empty_library: bool) -> Fixture {
    let mut s = GameScenario::new();
    s.at_phase(Phase::PreCombatMain);
    let cait = s
        .add_creature_from_oracle(P0, "Cait Sith, Fortune Teller", 3, 3, CAIT_SITH)
        .id();
    let bear = s.add_creature(P0, "Bear", 2, 2).id();
    let top = (!empty_library).then(|| {
        s.add_spell_to_library_top(P0, "Three Drop", false)
            .with_mana_cost(ManaCost::generic(3))
            .id()
    });
    Fixture {
        runner: s.build(),
        cait,
        bear,
        top,
    }
}

/// Advance from precombat main into combat, keeping the scried card on top,
/// until the reflexive's target prompt opens (returns true) or combat reaches
/// declare attackers with an empty stack (returns false).
fn advance_to_reflexive_prompt(r: &mut GameRunner) -> (bool, bool) {
    let mut saw_scry = false;
    for _ in 0..24 {
        match r.state().waiting_for.clone() {
            WaitingFor::ScryChoice { .. } => {
                saw_scry = true;
                r.act(GameAction::SelectCards { cards: vec![] })
                    .expect("keep the scried card on top");
            }
            WaitingFor::TriggerTargetSelection { .. } => return (saw_scry, true),
            WaitingFor::DeclareAttackers { .. } => break,
            WaitingFor::Priority { .. } => {
                if r.state().phase == Phase::DeclareAttackers && r.state().stack.is_empty() {
                    break;
                }
                r.act(GameAction::PassPriority).expect("pass");
            }
            other => panic!("unexpected prompt {other:?}"),
        }
    }
    (saw_scry, false)
}

fn resolve_stack(r: &mut GameRunner) {
    for _ in 0..8 {
        if r.state().stack.is_empty() {
            return;
        }
        r.act(GameAction::PassPriority).expect("pass");
    }
}

fn choose(r: &mut GameRunner, target: ObjectId) {
    r.act(GameAction::ChooseTarget {
        target: Some(TargetRef::Object(target)),
    })
    .expect("choose the reflexive's target");
}

#[test]
fn cait_sith_reflexive_pump_targets_after_the_exile() {
    let Fixture {
        mut runner,
        bear,
        top,
        ..
    } = fixture(false);
    let top = top.unwrap();
    let (saw_scry, prompted) = advance_to_reflexive_prompt(&mut runner);
    assert!(saw_scry, "reach: the combat trigger resolved its scry");
    // CR 603.12 + CR 603.3d: the target is chosen after the exile, for the
    // reflexive's own stack object.
    assert!(prompted, "the reflexive's target prompt opened");
    assert_eq!(zone(&runner, top), Zone::Exile, "the exile happened first");
    choose(&mut runner, bear);
    resolve_stack(&mut runner);
    assert_eq!(
        runner.state().objects[&bear].power,
        Some(5),
        "+X/+0 where X is the exiled card's mana value (3)"
    );
    // CR 608.2c: the permission granted before the reflexive still stands.
    assert!(
        !runner.state().objects[&top].casting_permissions.is_empty(),
        "the exiled card stays playable this turn"
    );
}

#[test]
fn cait_sith_with_nothing_exiled_creates_no_reflexive() {
    // Reach guard: the printed trigger is the supported reflexive shape, so the
    // missing prompt can't come from a fail-closed parse.
    let parsed = parse_oracle_text(
        CAIT_SITH,
        "Cait Sith, Fortune Teller",
        &[],
        &["Creature".to_string()],
        &[],
    );
    let execute = parsed.triggers[0].execute.as_deref().expect("trigger body");
    let mut node = Some(execute);
    let mut gated = false;
    while let Some(current) = node {
        assert!(
            !matches!(current.effect.as_ref(), Effect::Unimplemented { .. }),
            "supported chain: {execute:?}"
        );
        gated |= current.condition.as_ref().is_some_and(|condition| {
            condition.has_when_you_do_marker() && condition.zone_changed_this_way_head().is_some()
        });
        node = current.sub_ability.as_deref();
    }
    assert!(gated, "the pump is gated by the zone-change reflexive");

    let Fixture {
        mut runner,
        bear,
        cait,
        ..
    } = fixture(true);
    // Drive to declare attackers, recording the trigger and its exile step.
    let mut saw_trigger = false;
    let mut events = Vec::new();
    for _ in 0..24 {
        match runner.state().waiting_for.clone() {
            WaitingFor::ScryChoice { .. } => {
                let result = runner
                    .act(GameAction::SelectCards { cards: vec![] })
                    .expect("scry");
                events.extend(result.events);
            }
            WaitingFor::Priority { .. } => {
                if runner.state().phase == Phase::DeclareAttackers
                    && runner.state().stack.is_empty()
                {
                    break;
                }
                saw_trigger |= runner
                    .state()
                    .stack
                    .iter()
                    .any(|entry| entry.source_id == cait);
                let result = runner.act(GameAction::PassPriority).expect("pass");
                events.extend(result.events);
            }
            WaitingFor::DeclareAttackers { .. } => break,
            other => panic!("unexpected prompt {other:?}"),
        }
    }
    assert!(saw_trigger, "reach: Lucky Slots went on the stack");
    assert!(
        events.iter().any(|event| matches!(
            event,
            GameEvent::EffectResolved {
                kind: EffectKind::ExileTop,
                source_id,
                ..
            } if *source_id == cait
        )),
        "reach: the exile instruction resolved, exiling nothing"
    );
    // CR 603.12: no exile happened, so the reflexive never triggers.
    assert!(runner.state().stack.is_empty());
    assert_eq!(
        runner.state().phase,
        Phase::DeclareAttackers,
        "reach: combat advanced past the trigger"
    );
    assert_eq!(runner.state().objects[&bear].power, Some(2), "no pump");
}

#[test]
fn cait_sith_reflexive_fizzles_when_its_target_leaves() {
    let Fixture {
        mut runner,
        bear,
        top,
        ..
    } = fixture(false);
    let (_, prompted) = advance_to_reflexive_prompt(&mut runner);
    assert!(prompted, "reach: the reflexive's target prompt opened");
    choose(&mut runner, bear);
    assert_eq!(
        runner.state().stack.len(),
        1,
        "reach: the reflexive is on the stack"
    );
    let mut events = Vec::new();
    move_to_zone(runner.state_mut(), bear, Zone::Graveyard, &mut events);
    resolve_stack(&mut runner);
    // CR 608.2b: its only target is illegal, so it does nothing.
    assert!(runner.state().stack.is_empty());
    assert_eq!(zone(&runner, top.unwrap()), Zone::Exile);
}

#[test]
fn cait_sith_reflexive_reads_the_exiled_card_after_it_leaves_exile() {
    let Fixture {
        mut runner,
        bear,
        top,
        cait,
    } = fixture(false);
    let top = top.unwrap();
    let (_, prompted) = advance_to_reflexive_prompt(&mut runner);
    assert!(prompted, "reach: the reflexive's target prompt opened");
    choose(&mut runner, bear);
    // Hostile: the exiled card and Cait Sith both leave before the reflexive
    // resolves. CR 608.2h: "that card's mana value" is the last known
    // information of the card exiled this way; the reflexive is independent of
    // its source (CR 113.7a).
    let mut events = Vec::new();
    move_to_zone(runner.state_mut(), top, Zone::Hand, &mut events);
    move_to_zone(runner.state_mut(), cait, Zone::Graveyard, &mut events);
    resolve_stack(&mut runner);
    assert_eq!(runner.state().objects[&bear].power, Some(5));
}

#[test]
fn cait_sith_reflexive_survives_save_and_restore_at_its_target_prompt() {
    let Fixture {
        runner, bear, top, ..
    } = fixture(false);
    let mut runner = runner;
    let (_, prompted) = advance_to_reflexive_prompt(&mut runner);
    assert!(prompted, "reach: the reflexive's target prompt opened");
    let json = serde_json::to_string(&PersistedGameState::capture(runner.state().clone()))
        .expect("state serializes");
    let state = serde_json::from_str::<PersistedGameState>(&json)
        .expect("persisted state decodes")
        .prepare_for_restore(PersistedRestoreFinalization::DeferUntilRehydrated)
        .expect("persisted state is admissible")
        .finalize_after_rehydration(|_| Ok(()))
        .expect("restored state is publishable");
    let mut restored = GameRunner::from_state(state);
    assert!(matches!(
        restored.state().waiting_for,
        WaitingFor::TriggerTargetSelection { .. }
    ));
    choose(&mut restored, bear);
    resolve_stack(&mut restored);
    assert_eq!(restored.state().objects[&bear].power, Some(5));
    assert_eq!(zone(&restored, top.unwrap()), Zone::Exile);
}
