//! Auto-pay prefers a source's drawback-free mana ability when it suffices.
//!
//! A permanent with a plain `{T}: Add …` mana ability and a multi-mana
//! ability that costs more (a mana sub-cost, exert, sacrifice) must not have
//! the costlier ability chosen for it when a plan without it pays the whole
//! cost. When only the costlier ability pays the cost, auto-pay still uses it.
//!
//! CR 601.2g + CR 605.3b: mana abilities are activated during payment and
//! resolve immediately. CR 701.43a: exerting is the player's choice ("you
//! choose to have it not untap during your next untap step"); the automatic
//! payer must not make that choice, or a sacrifice, when it isn't needed.

use engine::game::mana_sources::activatable_mana_actions_for_player;
use engine::game::scenario::{GameRunner, GameScenario, P0, P1};
use engine::types::ability::{ContinuousModification, TargetFilter};
use engine::types::actions::GameAction;
use engine::types::events::GameEvent;
use engine::types::game_state::{CastPaymentMode, GameState, WaitingFor};
use engine::types::identifiers::ObjectId;
use engine::types::mana::{ManaColor, ManaCost, ManaCostShard, ManaType};
use engine::types::phase::Phase;
use engine::types::statics::StaticMode;
use engine::types::zones::Zone;

const ARENA_OF_GLORY_ORACLE: &str = "This land enters tapped unless you control a Mountain.\n\
{T}: Add {R}.\n\
{R}, {T}, Exert this land: Add {R}{R}. If any of that mana is spent on a creature spell, \
it gains haste until end of turn. (An exerted permanent won't untap during your next untap step.)";
const DWARVEN_RUINS_ORACLE: &str =
    "This land enters tapped.\n{T}: Add {R}.\n{T}, Sacrifice this land: Add {R}{R}.";
const OASIS_RITUALIST_ORACLE: &str = "{T}: Add one mana of any color.\n\
{T}, Exert this creature: Add two mana of any one color. \
(An exerted creature won't untap during your next untap step.)";
const TWILIGHT_MIRE_ORACLE: &str = "{T}: Add {C}.\n{B/G}, {T}: Add {B}{B}, {B}{G}, or {G}{G}.";
const AZORIUS_SIGNET_ORACLE: &str = "{1}, {T}: Add {W}{U}.";
const GRAVEN_CAIRNS_ORACLE: &str = "{T}: Add {C}.\n{B/R}, {T}: Add {B}{B}, {B}{R}, or {R}{R}.";
const REST_IN_PEACE_ORACLE: &str = "When this enchantment enters, exile all graveyards.\n\
If a card or token would be put into a graveyard from anywhere, exile it instead.";
const LEYLINE_OF_THE_VOID_ORACLE: &str = "If this card is in your opening hand, you may begin the \
game with it on the battlefield.\n\
If a card would be put into an opponent's graveyard from anywhere, exile it instead.";
const FETID_HEATH_ORACLE: &str = "{T}: Add {C}.\n{W/B}, {T}: Add {W}{W}, {W}{B}, or {B}{B}.";

/// Transient `CantUntap` effects aimed at `id` (the shape an exert cost installs).
fn exert_markers(state: &GameState, id: ObjectId) -> usize {
    state
        .transient_continuous_effects
        .iter()
        .filter(|e| e.affected == TargetFilter::SpecificObject { id })
        .filter(|e| {
            e.modifications.iter().any(|m| {
                matches!(
                    m,
                    ContinuousModification::AddStaticMode {
                        mode: StaticMode::CantUntap
                    }
                )
            })
        })
        .count()
}

fn cost(shards: &[ManaCostShard], generic: u32) -> ManaCost {
    ManaCost::Cost {
        shards: shards.to_vec(),
        generic,
    }
}

/// A "You gain 1 life." sorcery in P0's hand costing `mana_cost`.
fn witness_spell(scenario: &mut GameScenario, mana_cost: ManaCost) -> ObjectId {
    scenario
        .add_spell_to_hand_from_oracle(P0, "Auto-pay Witness", false, "You gain 1 life.")
        .with_mana_cost(mana_cost)
        .id()
}

/// Build the runner and untap `lands` (some printed lands enter tapped).
fn build_untapped(scenario: GameScenario, lands: &[ObjectId]) -> GameRunner {
    let mut runner = scenario.build();
    for land in lands {
        runner.state_mut().objects.get_mut(land).unwrap().tapped = false;
    }
    runner
}

fn mana_added(events: &[GameEvent]) -> Vec<(ObjectId, ManaType)> {
    events
        .iter()
        .filter_map(|event| match event {
            GameEvent::ManaAdded {
                source_id,
                mana_type,
                ..
            } => Some((*source_id, *mana_type)),
            _ => None,
        })
        .collect()
}

/// Cast the witness through the default auto-pay path and assert it resolved.
fn auto_pay_and_resolve(
    runner: &mut GameRunner,
    spell: ObjectId,
) -> engine::game::scenario::CastOutcome {
    let outcome = runner.cast(spell).resolve();
    assert!(
        outcome.zone_of(spell) == Zone::Graveyard && outcome.life_delta(P0) == 1,
        "reach: the witness was auto-paid and resolved; zone {:?}, life delta {}, mana added {:?}",
        outcome.zone_of(spell),
        outcome.life_delta(P0),
        mana_added(outcome.events()),
    );
    outcome
}

// --- Arena of Glory: exert + {R} sub-cost, never needed while the plain ability pays ---

fn assert_arena_not_exerted(shards: &[ManaCostShard], generic: u32, extra_mountains: usize) {
    let mut scenario = GameScenario::new();
    scenario.at_phase(Phase::PreCombatMain);
    let arena = scenario
        .add_land_from_oracle(P0, "Arena of Glory", ARENA_OF_GLORY_ORACLE)
        .id();
    let mut lands = vec![arena];
    for _ in 0..=extra_mountains {
        lands.push(scenario.add_basic_land(P0, ManaColor::Red));
    }
    let spell = witness_spell(&mut scenario, cost(shards, generic));
    let mut runner = build_untapped(scenario, &lands);
    assert_eq!(
        exert_markers(runner.state(), arena),
        0,
        "reach: Arena starts unexerted"
    );

    let outcome = auto_pay_and_resolve(&mut runner, spell);
    let tapped = lands
        .iter()
        .filter(|land| outcome.state().objects[land].tapped)
        .count();
    assert_eq!(
        exert_markers(outcome.state(), arena),
        0,
        "auto-pay exerted Arena of Glory although its `{{T}}: Add {{R}}` pays the cost; \
         mana added {:?} [arena = {arena:?}]",
        mana_added(outcome.events()),
    );
    assert_eq!(
        tapped,
        shards.len() + generic as usize,
        "one land per mana; mana added {:?}",
        mana_added(outcome.events()),
    );
}

/// The triage repro: a one-red spell with Arena + Mountain taps one land and
/// never exerts Arena (it used to tap both and float a red).
#[test]
fn arena_of_glory_auto_pay_for_single_red_does_not_exert() {
    assert_arena_not_exerted(&[ManaCostShard::Red], 0, 0);
}

/// The Goblin Storm shape: {1}{R} with Arena + Mountain.
#[test]
fn arena_of_glory_auto_pay_for_one_and_red_does_not_exert() {
    assert_arena_not_exerted(&[ManaCostShard::Red], 1, 0);
}

/// {R}{R} with Arena + two Mountains: the exert row covers both shards, but
/// Arena's plain ability plus a Mountain pays the same cost without exerting.
#[test]
fn arena_of_glory_auto_pay_for_two_red_does_not_exert() {
    assert_arena_not_exerted(&[ManaCostShard::Red, ManaCostShard::Red], 0, 1);
}

/// X costs: X is announced before payment (CR 601.2f), so {X}{R} with X = 1
/// pays exactly like {1}{R}.
#[test]
fn arena_of_glory_auto_pay_for_x_spell_does_not_exert() {
    let mut scenario = GameScenario::new();
    scenario.at_phase(Phase::PreCombatMain);
    let arena = scenario
        .add_land_from_oracle(P0, "Arena of Glory", ARENA_OF_GLORY_ORACLE)
        .id();
    let mountain = scenario.add_basic_land(P0, ManaColor::Red);
    let spell = scenario
        .add_spell_to_hand_from_oracle(P0, "X Witness", false, "You gain X life.")
        .with_mana_cost(cost(&[ManaCostShard::X, ManaCostShard::Red], 0))
        .id();
    let mut runner = build_untapped(scenario, &[arena, mountain]);

    let outcome = runner.cast(spell).x(1).resolve();
    assert!(
        outcome.zone_of(spell) == Zone::Graveyard && outcome.life_delta(P0) == 1,
        "reach: the X = 1 witness resolved; zone {:?}, life delta {}, mana added {:?}",
        outcome.zone_of(spell),
        outcome.life_delta(P0),
        mana_added(outcome.events()),
    );
    assert!(outcome.state().objects[&arena].tapped && outcome.state().objects[&mountain].tapped);
    assert_eq!(
        exert_markers(outcome.state(), arena),
        0,
        "mana added {:?}",
        mana_added(outcome.events())
    );
}

// --- Dwarven Ruins: sacrifice row ---

/// {R} from Dwarven Ruins alone: its plain ability pays; it must not be sacrificed.
#[test]
fn dwarven_ruins_auto_pay_for_single_red_does_not_sacrifice() {
    let mut scenario = GameScenario::new();
    scenario.at_phase(Phase::PreCombatMain);
    let ruins = scenario
        .add_land_from_oracle(P0, "Dwarven Ruins", DWARVEN_RUINS_ORACLE)
        .id();
    let spell = witness_spell(&mut scenario, cost(&[ManaCostShard::Red], 0));
    let mut runner = build_untapped(scenario, &[ruins]);

    let outcome = auto_pay_and_resolve(&mut runner, spell);
    assert_eq!(
        outcome.zone_of(ruins),
        Zone::Battlefield,
        "Dwarven Ruins was sacrificed for a cost its `{{T}}: Add {{R}}` pays; mana added {:?}",
        mana_added(outcome.events()),
    );
    assert!(outcome.state().objects[&ruins].tapped);
}

/// {R}{R} from Dwarven Ruins alone: only the sacrifice row pays it, so
/// auto-pay still uses it.
#[test]
fn dwarven_ruins_auto_pay_sacrifices_when_only_the_sacrifice_row_pays() {
    let mut scenario = GameScenario::new();
    scenario.at_phase(Phase::PreCombatMain);
    let ruins = scenario
        .add_land_from_oracle(P0, "Dwarven Ruins", DWARVEN_RUINS_ORACLE)
        .id();
    let spell = witness_spell(
        &mut scenario,
        cost(&[ManaCostShard::Red, ManaCostShard::Red], 0),
    );
    let mut runner = build_untapped(scenario, &[ruins]);

    let outcome = auto_pay_and_resolve(&mut runner, spell);
    assert_eq!(outcome.zone_of(ruins), Zone::Graveyard);
}

// --- Oasis Ritualist: exert row on a creature ---

/// {G} from Oasis Ritualist alone: its plain any-color ability pays it, so
/// the exert ability is not used.
#[test]
fn oasis_ritualist_auto_pay_for_single_green_does_not_exert() {
    let mut scenario = GameScenario::new();
    scenario.at_phase(Phase::PreCombatMain);
    let ritualist = scenario
        .add_creature_from_oracle(P0, "Oasis Ritualist", 2, 4, OASIS_RITUALIST_ORACLE)
        .id();
    let spell = witness_spell(&mut scenario, cost(&[ManaCostShard::Green], 0));
    let mut runner = build_untapped(scenario, &[ritualist]);

    let outcome = auto_pay_and_resolve(&mut runner, spell);
    assert!(outcome.state().objects[&ritualist].tapped);
    assert_eq!(
        exert_markers(outcome.state(), ritualist),
        0,
        "mana added {:?}",
        mana_added(outcome.events())
    );
}

// --- Twilight Mire: mana sub-cost (filter land) ---

fn mire_scenario(spell_cost: ManaCost) -> (GameRunner, ObjectId, ObjectId, ObjectId) {
    let mut scenario = GameScenario::new();
    scenario.at_phase(Phase::PreCombatMain);
    let swamp = scenario.add_basic_land(P0, ManaColor::Black);
    let mire = scenario
        .add_land_from_oracle(P0, "Twilight Mire", TWILIGHT_MIRE_ORACLE)
        .id();
    let spell = witness_spell(&mut scenario, spell_cost);
    (build_untapped(scenario, &[swamp, mire]), swamp, mire, spell)
}

/// {1}{B} with Swamp + Twilight Mire: Swamp's {B} and Mire's `{T}: Add {C}`
/// pay it exactly, so the `{B/G}`-costed filter ability is not used.
#[test]
fn twilight_mire_auto_pay_uses_its_plain_ability_when_it_suffices() {
    let (mut runner, swamp, mire, spell) = mire_scenario(cost(&[ManaCostShard::Black], 1));
    let outcome = auto_pay_and_resolve(&mut runner, spell);
    let mut added = mana_added(outcome.events());
    added.sort();
    let mut expected = vec![(swamp, ManaType::Black), (mire, ManaType::Colorless)];
    expected.sort();
    assert_eq!(added, expected);
}

/// {G} with Swamp + Twilight Mire: only the filter ability makes green, so it
/// is still used, with the Swamp paying its `{B/G}`.
#[test]
fn twilight_mire_auto_pay_uses_its_filter_ability_when_only_it_pays() {
    let (mut runner, swamp, mire, spell) = mire_scenario(cost(&[ManaCostShard::Green], 0));
    let outcome = auto_pay_and_resolve(&mut runner, spell);
    let added = mana_added(outcome.events());
    assert_eq!(added.first(), Some(&(swamp, ManaType::Black)), "{added:?}");
    assert!(added.contains(&(mire, ManaType::Green)), "{added:?}");
    assert!(outcome.state().objects[&swamp].tapped && outcome.state().objects[&mire].tapped);
}

// --- A drawback-free plan that would starve another source's mana sub-cost ---

/// {W}{U}{R}{R} with Azorius Signet + Dwarven Ruins + Mountain. Without the
/// sacrifice, the Signet's {W}{U} plus the Ruins' and Mountain's {R} cover
/// the cost's colors but leave nothing to pay the Signet's `{1}`. The only
/// complete payment sacrifices the Ruins for {R}{R} and taps the Mountain
/// for the Signet's `{1}`, so auto-pay must still find it.
#[test]
fn drawback_free_plan_is_rejected_when_it_cannot_fund_a_mana_sub_cost() {
    let mut scenario = GameScenario::new();
    scenario.at_phase(Phase::PreCombatMain);
    let signet = scenario
        .add_artifact_from_oracle(P0, "Azorius Signet", AZORIUS_SIGNET_ORACLE)
        .id();
    let ruins = scenario
        .add_land_from_oracle(P0, "Dwarven Ruins", DWARVEN_RUINS_ORACLE)
        .id();
    let mountain = scenario.add_basic_land(P0, ManaColor::Red);
    let spell = witness_spell(
        &mut scenario,
        cost(
            &[
                ManaCostShard::White,
                ManaCostShard::Blue,
                ManaCostShard::Red,
                ManaCostShard::Red,
            ],
            0,
        ),
    );
    let mut runner = build_untapped(scenario, &[signet, ruins, mountain]);

    let outcome = auto_pay_and_resolve(&mut runner, spell);
    assert_eq!(outcome.zone_of(ruins), Zone::Graveyard);
    assert!(outcome.state().objects[&signet].tapped && outcome.state().objects[&mountain].tapped);
}

// --- Two filter lands whose sub-costs must not be funded independently ---

/// P0: Graven Cairns, Fetid Heath, Dwarven Ruins, Swamp (in that order), all
/// untapped, and Hearthfire Hobgoblin ({R/W}{R/W}{R/W}) in hand.
fn cairns_heath_board() -> (GameRunner, [ObjectId; 4], ObjectId) {
    let mut scenario = GameScenario::new();
    scenario.at_phase(Phase::PreCombatMain);
    let cairns = scenario
        .add_land_from_oracle(P0, "Graven Cairns", GRAVEN_CAIRNS_ORACLE)
        .id();
    let heath = scenario
        .add_land_from_oracle(P0, "Fetid Heath", FETID_HEATH_ORACLE)
        .id();
    let ruins = scenario
        .add_land_from_oracle(P0, "Dwarven Ruins", DWARVEN_RUINS_ORACLE)
        .id();
    let swamp = scenario.add_basic_land(P0, ManaColor::Black);
    let hobgoblin = scenario
        .add_creature_to_hand_from_oracle(P0, "Hearthfire Hobgoblin", 2, 2, "Double strike")
        .with_mana_cost(cost(
            &[
                ManaCostShard::RedWhite,
                ManaCostShard::RedWhite,
                ManaCostShard::RedWhite,
            ],
            0,
        ))
        .id();
    let lands = [cairns, heath, ruins, swamp];
    let runner = build_untapped(scenario, &lands);
    for land in lands {
        assert!(
            !runner.state().objects[&land].tapped,
            "reach: {land:?} starts untapped"
        );
    }
    for (land, abilities) in [(cairns, 2), (heath, 2), (ruins, 2)] {
        assert_eq!(
            runner.state().objects[&land].abilities.len(),
            abilities,
            "reach: {land:?} has its plain and its costed mana ability"
        );
    }
    (runner, lands, hobgoblin)
}

/// CR 601.2g + CR 605.3b: Cairns {R}{R} and Heath {W}{W} pay the Hobgoblin, and
/// each filter ability's hybrid sub-cost needs its own funding source. Auto-pay
/// must not fund Cairns' {B/R} with the Swamp that Heath's {W/B} needs: the
/// cast is affordable and resolves through the default automatic payment.
#[test]
fn two_filter_land_sub_costs_are_funded_together() {
    let (mut runner, [cairns, heath, _ruins, _swamp], hobgoblin) = cairns_heath_board();
    assert!(
        engine::ai_support::legal_actions(runner.state())
            .iter()
            .any(
                |action| matches!(action, GameAction::CastSpell { object_id, .. }
                if *object_id == hobgoblin)
            ),
        "Hearthfire Hobgoblin must be affordable under automatic payment"
    );

    let outcome = runner.cast(hobgoblin).resolve();
    let filter_outputs: Vec<(ObjectId, ManaType)> = mana_added(outcome.events())
        .into_iter()
        .filter(|(source, _)| *source == cairns || *source == heath)
        .collect();
    assert_eq!(
        filter_outputs,
        vec![
            (cairns, ManaType::Red),
            (cairns, ManaType::Red),
            (heath, ManaType::White),
            (heath, ManaType::White),
        ],
        "reach: the selected filter outputs are Cairns {{R}}{{R}}, then Heath {{W}}{{W}}; all mana \
         added {:?}",
        mana_added(outcome.events()),
    );
    assert_eq!(
        outcome.zone_of(hobgoblin),
        Zone::Battlefield,
        "mana added {:?}",
        mana_added(outcome.events())
    );
}

/// Manual control for the board above: Swamp funds Heath, Ruins' plain {R}
/// funds Cairns, and the Hobgoblin resolves without sacrificing the Ruins.
#[test]
fn two_filter_land_sub_costs_manual_control() {
    let (mut runner, [cairns, heath, ruins, swamp], hobgoblin) = cairns_heath_board();
    let activate = |runner: &mut GameRunner, source: ObjectId, output: &[ManaType]| {
        let action = activatable_mana_actions_for_player(runner.state(), P0)
            .into_iter()
            .find(|action| match action {
                GameAction::TapLandForMana { selection } => {
                    selection.source.object_id == source
                        && match output {
                            [single] => {
                                selection.atomic_combination.is_none()
                                    && selection.mana_type == *single
                            }
                            combination => {
                                selection.atomic_combination.as_deref() == Some(combination)
                            }
                        }
                }
                _ => false,
            })
            .unwrap_or_else(|| panic!("{source:?} offers {output:?}"));
        runner.act(action).expect("manual mana activation");
    };
    activate(&mut runner, swamp, &[ManaType::Black]);
    activate(&mut runner, heath, &[ManaType::White, ManaType::White]);
    activate(&mut runner, ruins, &[ManaType::Red]);
    activate(&mut runner, cairns, &[ManaType::Red, ManaType::Red]);

    let outcome = runner.cast(hobgoblin).resolve();
    assert_eq!(outcome.zone_of(hobgoblin), Zone::Battlefield);
    assert_eq!(outcome.zone_of(ruins), Zone::Battlefield);
}

// --- A payment that pauses keeps the payment's row policy when it resumes ---

/// CR 601.2g + CR 605.3b + CR 616.1: two Dwarven Ruins pay {R/W}{R/W}{R/W} only
/// with a sacrifice row, so the payment plans with every row (two sacrifices,
/// as before drawback avoidance). Rest in Peace and Leyline of the Void both
/// replace the first Ruins' trip to the graveyard, so the payment pauses for
/// P0's replacement choice. The resumed payment must keep the policy the
/// payment decided rather than re-deciding from what is left: the second Ruins
/// is sacrificed too.
#[test]
fn resumed_payment_keeps_its_row_policy() {
    let mut scenario = GameScenario::new();
    scenario.at_phase(Phase::PreCombatMain);
    let first_ruins = scenario
        .add_land_from_oracle(P0, "Dwarven Ruins", DWARVEN_RUINS_ORACLE)
        .id();
    let second_ruins = scenario
        .add_land_from_oracle(P0, "Dwarven Ruins", DWARVEN_RUINS_ORACLE)
        .id();
    scenario.add_enchantment_from_oracle(P1, "Rest in Peace", REST_IN_PEACE_ORACLE);
    scenario.add_enchantment_from_oracle(P1, "Leyline of the Void", LEYLINE_OF_THE_VOID_ORACLE);
    let hobgoblin = scenario
        .add_creature_to_hand_from_oracle(P0, "Hearthfire Hobgoblin", 2, 2, "Double strike")
        .with_mana_cost(cost(
            &[
                ManaCostShard::RedWhite,
                ManaCostShard::RedWhite,
                ManaCostShard::RedWhite,
            ],
            0,
        ))
        .id();
    let mut runner = build_untapped(scenario, &[first_ruins, second_ruins]);
    let card_id = runner.state().objects[&hobgoblin].card_id;

    runner
        .act(GameAction::CastSpell {
            object_id: hobgoblin,
            card_id,
            targets: vec![],
            payment_mode: CastPaymentMode::Auto,
        })
        .expect("the automatic cast starts paying");
    assert!(
        matches!(
            runner.state().waiting_for,
            WaitingFor::ReplacementChoice { .. }
        ),
        "reach: the first Ruins' sacrifice pauses for a replacement choice, got {:?}",
        runner.state().waiting_for
    );
    let tapped_before_choice = [first_ruins, second_ruins]
        .into_iter()
        .filter(|ruins| runner.state().objects[ruins].tapped)
        .count();
    assert_eq!(
        tapped_before_choice, 1,
        "reach: one Ruins has paid {{T}} and waits on its sacrifice; the other is untouched"
    );

    // Each sacrificed Ruins asks P0 to order Rest in Peace and Leyline of the
    // Void (CR 616.1). Answer every prompt the payment raises.
    let mut replacement_choices = 0;
    while matches!(
        runner.state().waiting_for,
        WaitingFor::ReplacementChoice { .. }
    ) {
        runner
            .act(GameAction::ChooseReplacement { index: 0 })
            .expect("the replacement choice resumes the payment");
        replacement_choices += 1;
        assert!(replacement_choices <= 2, "at most one choice per Ruins");
    }
    assert!(
        matches!(runner.state().waiting_for, WaitingFor::Priority { .. }),
        "the resumed payment completes the cast; waiting for {:?}",
        runner.state().waiting_for
    );
    assert_eq!(
        replacement_choices, 2,
        "the resumed payment keeps the every-row plan and sacrifices the second Ruins too"
    );
    for ruins in [first_ruins, second_ruins] {
        assert!(
            !runner.state().battlefield.contains(&ruins),
            "{ruins:?} was sacrificed"
        );
    }
    let exiled_ruins = runner
        .state()
        .exile
        .iter()
        .filter(|id| runner.state().objects[id].name == "Dwarven Ruins")
        .count();
    assert_eq!(
        exiled_ruins, 2,
        "reach: both sacrificed Ruins were exiled by the chosen replacement"
    );
    runner.resolve_top();
    assert!(
        runner.state().battlefield.contains(&hobgoblin),
        "reach: the paid Hobgoblin resolves"
    );
}
