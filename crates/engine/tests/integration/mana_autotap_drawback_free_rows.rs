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

use engine::game::scenario::{GameRunner, GameScenario, P0};
use engine::types::ability::{ContinuousModification, TargetFilter};
use engine::types::events::GameEvent;
use engine::types::game_state::GameState;
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
