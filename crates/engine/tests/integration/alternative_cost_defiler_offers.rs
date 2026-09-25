//! CR 601.2b + CR 118.9d: an alternative cost is offered when it is affordable
//! only after a matching Defiler's optional reduction.
//!
//! A Defiler ("As an additional cost to cast [color] permanent spells, you may
//! pay 2 life. Those spells cost {C} less to cast if you paid life this way.")
//! reduces the alternative cost being paid (CR 118.9d). The offer for an
//! alternative cost must count that reduction, as ordinary castability already
//! does; otherwise an alternative cost the player can afford is never offered.
//! Each test gives exactly enough mana for the REDUCED alternative cost, which
//! the printed cost can't be paid with even after the reduction.

use engine::game::scenario::{GameRunner, GameScenario, P0};
use engine::parser::oracle::parse_oracle_text;
use engine::types::actions::GameAction;
use engine::types::game_state::{CastPaymentMode, WaitingFor};
use engine::types::identifiers::ObjectId;
use engine::types::mana::{ManaColor, ManaCost, ManaCostShard, ManaType, ManaUnit};
use engine::types::phase::Phase;
use engine::types::zones::Zone;

const DEFILER_OF_DREAMS: &str = "Flying\nAs an additional cost to cast blue permanent spells, you may pay 2 life. Those spells cost {U} less to cast if you paid life this way. This effect reduces only the amount of blue mana you pay.\nWhenever you cast a blue permanent spell, draw a card.";

const DEFILER_OF_INSTINCT: &str = "First strike\nAs an additional cost to cast red permanent spells, you may pay 2 life. Those spells cost {R} less to cast if you paid life this way. This effect reduces only the amount of red mana you pay.\nWhenever you cast a red permanent spell, this creature deals 1 damage to any target.";

const MULLDRIFTER: &str = "Flying\nWhen this creature enters, draw two cards.\nEvoke {2}{U} (You may cast this spell for its evoke cost. If you do, it's sacrificed when it enters.)";

const GOBLIN_HEELCUTTER: &str = "Whenever this creature attacks, target creature can't block this turn.\nDash {2}{R} (You may cast this spell for its dash cost. If you do, it gains haste, and it's returned from the battlefield to its owner's hand at the beginning of the next end step.)";

/// Put a Defiler on P0's battlefield with the statics its Oracle text parses to
/// (not its cast trigger, which would add an unrelated prompt).
fn add_defiler(scenario: &mut GameScenario, name: &str, oracle: &str, keyword: &str) {
    let parsed = parse_oracle_text(oracle, name, &[keyword.into()], &["Creature".into()], &[]);
    assert!(
        parsed
            .statics
            .iter()
            .any(|s| format!("{s:?}").contains("DefilerCostReduction")),
        "{name} must parse to a Defiler cost reduction, got {:?}",
        parsed.statics
    );
    let mut defiler = scenario.add_creature(P0, name, 3, 3);
    for s in parsed.statics {
        defiler.with_static_definition(s);
    }
}

fn add_colorless_mana(runner: &mut GameRunner, amount: usize) {
    for _ in 0..amount {
        runner.state_mut().players[0].mana_pool.add(ManaUnit::new(
            ManaType::Colorless,
            ObjectId(0),
            false,
            vec![],
        ));
    }
}

/// Cast `spell` from hand, accept the Defiler, and return the life paid.
fn cast_accepting_the_defiler(runner: &mut GameRunner, spell: ObjectId) -> i32 {
    let life_before = runner.state().players[0].life;
    let card_id = runner.state().objects[&spell].card_id;
    runner
        .act(GameAction::CastSpell {
            object_id: spell,
            card_id,
            targets: vec![],
            payment_mode: CastPaymentMode::Auto,
        })
        .expect("the alternative cost is affordable with the Defiler, so the cast is legal");
    // Positive reach guard: the Defiler choice is reached.
    assert!(
        matches!(
            runner.state().waiting_for,
            WaitingFor::DefilerPayment { .. }
        ),
        "the matching Defiler must be offered, got {:?}",
        runner.state().waiting_for
    );
    runner
        .act(GameAction::DecideOptionalCost { pay: true })
        .expect("accepting the Defiler must complete the cast");
    life_before - runner.state().players[0].life
}

/// CR 702.74a + CR 118.9d: Mulldrifter's evoke {2}{U} less the Defiler's {U} is
/// {2}. With 2 colorless mana, evoke is affordable only with the Defiler, and
/// the printed {4}{U} is not affordable even with it.
#[test]
fn evoke_affordable_only_with_a_defiler_is_offered() {
    let mut scenario = GameScenario::new();
    scenario.at_phase(Phase::PreCombatMain);
    add_defiler(
        &mut scenario,
        "Defiler of Dreams",
        DEFILER_OF_DREAMS,
        "Flying",
    );
    let mulldrifter = scenario
        .add_creature_to_hand_from_oracle(P0, "Mulldrifter", 2, 2, MULLDRIFTER)
        .with_mana_cost(ManaCost::Cost {
            generic: 4,
            shards: vec![ManaCostShard::Blue],
        })
        .with_color(vec![ManaColor::Blue])
        .id();
    let mut runner = scenario.build();
    add_colorless_mana(&mut runner, 2);

    let life_paid = cast_accepting_the_defiler(&mut runner, mulldrifter);

    assert_eq!(runner.state().objects[&mulldrifter].zone, Zone::Stack);
    assert_eq!(life_paid, 2, "the Defiler's 2 life must be paid");
    assert_eq!(
        runner.state().players[0].mana_pool.total(),
        0,
        "evoke {{2}}{{U}} less {{U}} costs exactly the 2 mana available"
    );
}

/// CR 702.109a + CR 118.9d: Goblin Heelcutter's dash {2}{R} less the Defiler's
/// {R} is {2}. With 2 colorless mana, dash is affordable only with the Defiler,
/// and the printed {3}{R} is not affordable even with it.
#[test]
fn dash_affordable_only_with_a_defiler_is_offered() {
    let mut scenario = GameScenario::new();
    scenario.at_phase(Phase::PreCombatMain);
    add_defiler(
        &mut scenario,
        "Defiler of Instinct",
        DEFILER_OF_INSTINCT,
        "First strike",
    );
    let heelcutter = scenario
        .add_creature_to_hand_from_oracle(P0, "Goblin Heelcutter", 3, 2, GOBLIN_HEELCUTTER)
        .with_mana_cost(ManaCost::Cost {
            generic: 3,
            shards: vec![ManaCostShard::Red],
        })
        .with_color(vec![ManaColor::Red])
        .id();
    let mut runner = scenario.build();
    add_colorless_mana(&mut runner, 2);

    let life_paid = cast_accepting_the_defiler(&mut runner, heelcutter);

    assert_eq!(runner.state().objects[&heelcutter].zone, Zone::Stack);
    assert_eq!(life_paid, 2, "the Defiler's 2 life must be paid");
    assert_eq!(
        runner.state().players[0].mana_pool.total(),
        0,
        "dash {{2}}{{R}} less {{R}} costs exactly the 2 mana available"
    );
}
