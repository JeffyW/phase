//! CR 601.2f + CR 602.2b: the cost-reduction election for ACTIVATED abilities.
//!
//! CR 602.2b makes an activation cost the analog of a spell's mana cost for
//! CR 601.2f: "The total cost is the mana cost or alternative cost ..., plus all
//! additional costs and cost increases, and minus all cost reductions. If
//! multiple cost reductions apply, the player may apply them in any order."
//!
//! Activation reductions are generic-only (CR 118.7a), so the order matters for
//! one reason: a floor. Training Grounds "can't reduce the mana in that cost to
//! less than one mana", so on `{3}` Training Grounds (−2) then an unfloored −2
//! locks `{0}`, while the reverse locks `{1}`. Reductions with equal effective
//! floors commute.
//!
//! These tests pin the substrate the election sits on: raises are applied before
//! reductions (CR 601.2f), nothing depends on battlefield order, and the default
//! order — the one every preview uses — is the cheapest one.

use engine::game::casting::can_activate_ability_now;
use engine::game::scenario::{GameRunner, GameScenario, P0};
use engine::types::ability::{
    AbilityCost, AbilityDefinition, AbilityKind, ControllerRef, CostReduction, Effect,
    QuantityExpr, StaticDefinition, TargetFilter, TypedFilter,
};
use engine::types::actions::GameAction;
use engine::types::identifiers::ObjectId;
use engine::types::mana::{ManaCost, ManaType, ManaUnit};
use engine::types::phase::Phase;
use engine::types::statics::{ActivationExemption, CostModifyMode, StaticMode};

/// Scryfall-verified Oracle text.
const TRAINING_GROUNDS: &str = "Activated abilities of creatures you control cost {2} less to activate. This effect can't reduce the mana in that cost to less than one mana.";
const SUPPRESSION_FIELD: &str =
    "Activated abilities cost {2} more to activate unless they're mana abilities.";

/// One cost modifier on the battlefield.
#[derive(Clone, Copy, Debug)]
enum Modifier {
    /// Training Grounds: −2, can't reduce below one mana.
    Grounds,
    /// Suppression Field: +2, mana abilities exempt.
    Suppression,
    /// An unfloored "activated abilities of creatures you control cost {N} less"
    /// reducer — the Professor Hojo shape, which no printed card on this tree
    /// carries yet, so it is built directly.
    Unfloored(u32),
}

fn colorless(n: usize) -> Vec<ManaUnit> {
    (0..n)
        .map(|_| ManaUnit::new(ManaType::Colorless, ObjectId(0), false, vec![]))
        .collect()
}

fn unfloored_reducer(amount: u32) -> StaticDefinition {
    StaticDefinition::new(StaticMode::ReduceAbilityCost {
        mode: CostModifyMode::Reduce,
        keyword: "activated".to_string(),
        amount,
        minimum_mana: None,
        dynamic_count: None,
        exemption: ActivationExemption::None,
        activator: None,
    })
    .affected(TargetFilter::Typed(
        TypedFilter::creature().controller(ControllerRef::You),
    ))
}

struct Board {
    runner: GameRunner,
    source: ObjectId,
}

impl Board {
    /// Modifiers are created in the order given, so reversing the slice reverses
    /// battlefield order. `rider` is the ability's own "costs {N} less" text.
    fn new(modifiers: &[Modifier], cost: AbilityCost, rider: Option<u32>, pool: usize) -> Self {
        let mut scenario = GameScenario::new();
        scenario.at_phase(Phase::PreCombatMain);
        for (i, modifier) in modifiers.iter().enumerate() {
            match modifier {
                Modifier::Grounds => {
                    scenario.add_enchantment_from_oracle(
                        P0,
                        &format!("Training Grounds {i}"),
                        TRAINING_GROUNDS,
                    );
                }
                Modifier::Suppression => {
                    scenario.add_enchantment_from_oracle(
                        P0,
                        &format!("Suppression Field {i}"),
                        SUPPRESSION_FIELD,
                    );
                }
                Modifier::Unfloored(amount) => {
                    scenario
                        .add_creature(P0, &format!("Reducer {i}"), 1, 1)
                        .with_static_definition(unfloored_reducer(*amount));
                }
            }
        }
        let mut ability = AbilityDefinition::new(
            AbilityKind::Activated,
            Effect::GainLife {
                amount: QuantityExpr::Fixed { value: 1 },
                player: TargetFilter::Controller,
            },
        )
        .cost(cost);
        if let Some(amount_per) = rider {
            ability.cost_reduction = Some(CostReduction {
                mode: CostModifyMode::Reduce,
                amount_per,
                count: QuantityExpr::Fixed { value: 1 },
                condition: None,
            });
        }
        let source = scenario
            .add_creature(P0, "Activator", 2, 2)
            .with_ability_definition(ability)
            .id();
        scenario.with_mana_pool(P0, colorless(pool));
        Self {
            runner: scenario.build(),
            source,
        }
    }

    fn pool(&self) -> usize {
        self.runner.state().players[0].mana_pool.mana.len()
    }

    /// Activate and report the mana it spent, asserting it reached the stack.
    fn activate_and_pay(&mut self) -> usize {
        let before = self.pool();
        self.runner
            .act(GameAction::ActivateAbility {
                source_id: self.source,
                ability_index: 0,
            })
            .expect("the activation must be legal");
        assert!(
            self.runner
                .state()
                .stack
                .iter()
                .any(|entry| entry.source_id == self.source),
            "the activation must reach the stack"
        );
        before - self.pool()
    }
}

fn generic(n: u32) -> AbilityCost {
    AbilityCost::Mana {
        cost: ManaCost::generic(n),
    }
}

/// Reach guard for every Oracle-text fixture: the parse must yield the floor and
/// the raise these tests rely on, or a mis-parse would silently test nothing.
#[test]
fn the_printed_modifiers_parse_to_the_shapes_under_test() {
    let board = Board::new(
        &[Modifier::Grounds, Modifier::Suppression],
        generic(3),
        None,
        0,
    );
    let modes: Vec<StaticMode> = board
        .runner
        .state()
        .objects
        .values()
        .flat_map(|obj| {
            obj.static_definitions
                .as_slice()
                .iter()
                .map(|d| d.mode.clone())
        })
        .collect();
    assert!(
        modes.iter().any(|mode| matches!(
            mode,
            StaticMode::ReduceAbilityCost {
                mode: CostModifyMode::Reduce,
                amount: 2,
                minimum_mana: Some(1),
                ..
            }
        )),
        "Training Grounds must parse to a floored -2, got {modes:?}"
    );
    assert!(
        modes.iter().any(|mode| matches!(
            mode,
            StaticMode::ReduceAbilityCost {
                mode: CostModifyMode::Raise,
                amount: 2,
                exemption: ActivationExemption::ManaAbilities,
                ..
            }
        )),
        "Suppression Field must parse to a +2 raise exempting mana abilities, got {modes:?}"
    );
}

/// CR 601.2f: raises are added before reductions. Before this, application
/// followed battlefield order, so `{1}` under Suppression Field and Training
/// Grounds cost 1 or 3 depending on which entered first. 3 is not a total any
/// legal order produces: 1 + 2 = 3, and Training Grounds then takes it to 1.
#[test]
fn a_raise_is_applied_before_a_floored_reduction_in_either_battlefield_order() {
    for modifiers in [
        [Modifier::Suppression, Modifier::Grounds],
        [Modifier::Grounds, Modifier::Suppression],
    ] {
        let mut board = Board::new(&modifiers, generic(1), None, 5);
        assert_eq!(
            board.activate_and_pay(),
            1,
            "{modifiers:?}: {{1}} + {{2}} - {{2}} (floor one mana) must lock {{1}}"
        );
    }
}

/// CR 601.2f: the ability's own rider is one reduction among the others, not a
/// step that always runs first. It used to be hard-wired first, which is the more
/// expensive order here: `{3}` −2 then Training Grounds is `{1}`, while Training
/// Grounds then −2 is `{0}`. The default is now the cheapest order.
#[test]
fn the_abilitys_own_rider_joins_the_default_order() {
    let mut board = Board::new(&[Modifier::Grounds], generic(3), Some(2), 5);
    assert_eq!(board.activate_and_pay(), 0);
}

/// CR 601.2f: the default order does not depend on battlefield order, and it is
/// the cheapest order — Training Grounds before the unfloored reducer.
#[test]
fn the_default_order_is_the_cheapest_in_either_battlefield_order() {
    for modifiers in [
        [Modifier::Grounds, Modifier::Unfloored(2)],
        [Modifier::Unfloored(2), Modifier::Grounds],
    ] {
        let mut board = Board::new(&modifiers, generic(3), None, 5);
        assert_eq!(board.activate_and_pay(), 0, "{modifiers:?}");
    }
}

/// Reductions with equal effective floors commute, so battlefield order changes
/// nothing — and they really did apply (each total is below the printed `{3}`).
#[test]
fn equal_floors_commute() {
    for modifiers in [
        [Modifier::Unfloored(1), Modifier::Unfloored(2)],
        [Modifier::Unfloored(2), Modifier::Unfloored(1)],
    ] {
        let mut board = Board::new(&modifiers, generic(3), None, 5);
        assert_eq!(board.activate_and_pay(), 0, "{modifiers:?}");
    }
    let mut board = Board::new(&[Modifier::Grounds, Modifier::Grounds], generic(3), None, 5);
    assert_eq!(
        board.activate_and_pay(),
        1,
        "two floored reductions stop at one mana"
    );
}

/// CR 606.1: a reduction cannot touch a bare loyalty cost, so a loyalty ability
/// under reducers keeps the mana-free fast path and pays only loyalty.
#[test]
fn a_bare_loyalty_cost_is_untouched_by_reductions() {
    let mut scenario = GameScenario::new();
    scenario.at_phase(Phase::PreCombatMain);
    for amount in [1, 2] {
        let mut reducer = unfloored_reducer(amount);
        reducer.affected = None;
        if let StaticMode::ReduceAbilityCost { keyword, .. } = &mut reducer.mode {
            *keyword = "loyalty".to_string();
        }
        scenario
            .add_creature(P0, "Loyalty reducer", 1, 1)
            .with_static_definition(reducer);
    }
    let walker = scenario
        .add_planeswalker_from_oracle(P0, "Test Walker", "Test", 3, "+1: You gain 1 life.")
        .id();
    scenario.with_mana_pool(P0, colorless(5));
    let mut runner = scenario.build();
    let index = runner.state().objects[&walker]
        .abilities
        .iter()
        .position(|a| matches!(a.kind, AbilityKind::Activated))
        .expect("the planeswalker has a loyalty ability");
    runner
        .act(GameAction::ActivateAbility {
            source_id: walker,
            ability_index: index,
        })
        .expect("the loyalty ability must activate");
    assert_eq!(
        runner.state().players[0].mana_pool.mana.len(),
        5,
        "no mana paid"
    );
    assert_eq!(runner.state().objects[&walker].loyalty, Some(4));
    assert_eq!(runner.state().stack.len(), 1);
}

/// The preview reads the default fold, which is the cheapest order, so an
/// activation that some electable order makes free is offered with no mana —
/// in the battlefield order that used to make it cost one.
#[test]
fn the_preview_offers_an_activation_the_cheapest_order_makes_free() {
    let board = Board::new(
        &[Modifier::Unfloored(2), Modifier::Grounds],
        generic(3),
        None,
        0,
    );
    assert!(can_activate_ability_now(
        board.runner.state(),
        P0,
        board.source,
        0
    ));
}
