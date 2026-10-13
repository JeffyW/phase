//! CR 603.12 + CR 701.20a: a "When you reveal a <filter> card this way"
//! reflexive is lowered (`WhenYouDo` + the reveal-until-hit guard) only when the
//! reveal-until before it is proven to be the resolution's only instruction and
//! to execute exactly once. The guard reads that one reveal's own verdict; with
//! a second reveal or a repeated process, the antecedent and its occurrence set
//! are not proven by this lowering, so the head stays explicitly unsupported
//! (issue #9763).

use engine::game::scenario::{GameRunner, GameScenario, P0};
use engine::types::ability::{
    AbilityCondition, AbilityDefinition, Effect, EffectOutcomeSignal, QuantityExpr,
};
use engine::types::actions::GameAction;
use engine::types::game_state::WaitingFor;
use engine::types::identifiers::ObjectId;
use engine::types::mana::ManaCost;
use engine::types::phase::Phase;
use engine::types::zones::Zone;

const HEAD: &str = "Reveal cards from the top of your library until you reveal a creature card. Put that card into your hand and the rest on the bottom of your library in a random order. ";
const GATE: &str = "When you reveal a creature card this way, you gain 3 life.";

/// The issue's board: two reveal-untils before the head.
const DOUBLE_DIG: &str = "Reveal cards from the top of your library until you reveal a land card. Put that card into your hand and the rest on the bottom of your library in a random order. Reveal cards from the top of your library until you reveal a creature card. Put that card into your hand and the rest on the bottom of your library in a random order. When you reveal a creature card this way, you gain 3 life.";

/// `creatures` creature cards on top of P0's library.
fn board(oracle: &str, creatures: usize) -> (GameRunner, ObjectId, Vec<ObjectId>) {
    let mut scenario = GameScenario::new();
    scenario.at_phase(Phase::PreCombatMain);
    let spell = scenario
        .add_spell_to_hand_from_oracle(P0, "Probe Dig", true, oracle)
        .with_mana_cost(ManaCost::zero())
        .id();
    let library = (0..creatures)
        .map(|_| {
            scenario
                .add_spell_to_library_top(P0, "Grizzly Bears", false)
                .as_creature()
                .with_mana_cost(ManaCost::generic(2))
                .id()
        })
        .collect();
    (scenario.build(), spell, library)
}

fn root(runner: &GameRunner, spell: ObjectId) -> AbilityDefinition {
    let abilities = &runner.state().objects[&spell].abilities;
    assert_eq!(abilities.len(), 1, "{abilities:?}");
    abilities[0].clone()
}

fn chain(def: &AbilityDefinition) -> Vec<&AbilityDefinition> {
    let mut nodes = vec![def];
    while let Some(next) = nodes.last().and_then(|node| node.sub_ability.as_deref()) {
        nodes.push(next);
    }
    nodes
}

fn reveal_untils(def: &AbilityDefinition) -> Vec<&AbilityDefinition> {
    chain(def)
        .into_iter()
        .filter(|node| matches!(node.effect.as_ref(), Effect::RevealUntil { .. }))
        .collect()
}

fn has_reveal_gate_gap(def: &AbilityDefinition) -> bool {
    chain(def).into_iter().any(|node| {
        matches!(
            node.effect.as_ref(),
            Effect::Unimplemented { name, .. } if name == "reveal_this_way_reflexive_gate"
        )
    })
}

fn carries_the_guard(def: &AbilityDefinition) -> bool {
    let guard = AbilityCondition::when_you_do_with_guard(AbilityCondition::EffectOutcome {
        signal: EffectOutcomeSignal::RevealUntilMatched,
    });
    chain(def)
        .into_iter()
        .any(|node| node.condition.as_ref() == Some(&guard))
}

fn resolve(runner: &mut GameRunner) {
    for _ in 0..16 {
        match runner.state().waiting_for.clone() {
            WaitingFor::Priority { .. } if !runner.state().stack.is_empty() => {
                runner.act(GameAction::PassPriority).expect("pass");
            }
            WaitingFor::Priority { .. } => return,
            other => panic!("unexpected prompt {other:?}"),
        }
    }
    panic!("the stack never emptied");
}

/// Two reveal-untils before the head: the guard could read only the second's
/// verdict, while "this way" may name what the first revealed too.
#[test]
fn a_second_reveal_until_before_the_head_fails_closed() {
    let (runner, spell, _) = board(DOUBLE_DIG, 0);
    let def = root(&runner, spell);
    assert_eq!(
        reveal_untils(&def).len(),
        2,
        "reach guard: both reveal-untils parsed: {def:?}"
    );
    assert!(has_reveal_gate_gap(&def), "{def:?}");
    assert!(!carries_the_guard(&def), "{def:?}");
}

/// Probe P1: "Repeat this process one more time" before the head stamps a
/// total count of 2 on the one reveal-until clause without emitting another
/// clause — the reveal runs twice, and the guard would see only the last
/// verdict.
#[test]
fn a_repeated_reveal_before_the_head_fails_closed() {
    let (runner, spell, _) = board(
        &format!("{HEAD}Repeat this process one more time. {GATE}"),
        2,
    );
    let def = root(&runner, spell);
    let reveals = reveal_untils(&def);
    assert_eq!(reveals.len(), 1, "reach guard: one reveal-until: {def:?}");
    assert!(
        matches!(
            reveals[0].effect.as_ref(),
            Effect::RevealUntil {
                count: QuantityExpr::Fixed { value: 1 },
                ..
            }
        ),
        "reach guard: count 1: {def:?}"
    );
    assert!(
        matches!(
            reveals[0].repeat_for,
            Some(QuantityExpr::Offset { offset: 1, .. })
        ),
        "reach guard: the process runs once more (1 + 1): {def:?}"
    );
    assert!(has_reveal_gate_gap(&def), "{def:?}");
    assert!(!carries_the_guard(&def), "{def:?}");
}

/// A repeat directive printed after the head repeats the whole process,
/// reveal included; the gate is already lowered when the directive is read,
/// so the proof runs over the finished chain.
#[test]
fn a_repeat_directive_after_the_head_fails_closed() {
    for directive in ["Repeat this process one more time.", "Repeat this process."] {
        let (runner, spell, _) = board(&format!("{HEAD}{GATE} {directive}"), 0);
        let def = root(&runner, spell);
        assert_eq!(
            reveal_untils(&def).len(),
            1,
            "{directive}: reach guard: one reveal-until: {def:?}"
        );
        assert!(has_reveal_gate_gap(&def), "{directive}: {def:?}");
        assert!(!carries_the_guard(&def), "{directive}: {def:?}");
    }
}

/// Control for P1: the same text without the repeat still lowers to the
/// guarded reflexive and resolves (CR 603.12).
#[test]
fn the_single_reveal_still_lowers_and_resolves() {
    let (mut runner, spell, library) = board(&format!("{HEAD}{GATE}"), 2);
    let def = root(&runner, spell);
    assert!(carries_the_guard(&def), "{def:?}");
    assert!(!has_reveal_gate_gap(&def), "{def:?}");
    runner.cast(spell).commit();
    resolve(&mut runner);
    let in_hand = library
        .iter()
        .filter(|id| runner.state().objects[*id].zone == Zone::Hand)
        .count();
    assert_eq!(in_hand, 1, "reach guard: the reveal hit");
    assert_eq!(runner.life(P0), 23);
}
