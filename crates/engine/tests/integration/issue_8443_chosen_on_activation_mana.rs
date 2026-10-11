//! Issue #8443: a mana ability that chooses its color during activation
//! ("Choose a color. Add ... of that color") must count as mana when deciding
//! whether a spell can be cast, whether auto-pass skips the priority window,
//! and how auto-tap pays.
//!
//! CR 106.1 + CR 607.2d: "the chosen color" on Sunken Citadel is linked to its
//! as-enters choice, and an unset linked choice produces no mana (CR 106.5).
//! Astral Cornucopia, Nykthos and Nyx Lotus have no linked chooser; the color
//! is chosen while the ability resolves, so any color is available.

use engine::ai_support::{auto_pass_recommended, flat_priority_actions, legal_actions};
use engine::game::scenario::{GameRunner, GameScenario, P0, P1};
use engine::types::ability::{
    AbilityCondition, AbilityCost, AbilityDefinition, AbilityKind, ChoiceType, ChosenAttribute,
    ContinuousModification, Duration, Effect, ManaProduction, PlayerFilter, QuantityExpr,
    ReplacementDefinition, StaticDefinition, TargetFilter, TargetSelectionMode,
};
use engine::types::actions::GameAction;
use engine::types::counter::CounterType;
use engine::types::game_state::{CastPaymentMode, ManaChoice, WaitingFor};
use engine::types::identifiers::ObjectId;
use engine::types::mana::{ManaColor, ManaCost, ManaCostShard, ManaType, ManaUnit};
use engine::types::phase::Phase;
use engine::types::replacements::ReplacementEvent;
use engine::types::zones::Zone;

const ASTRAL: &str = "This artifact enters with X charge counters on it.\n{T}: Choose a color. Add one mana of that color for each charge counter on this artifact.";
const NYKTHOS: &str = "{T}: Add {C}.\n{2}, {T}: Choose a color. Add an amount of mana of that color equal to your devotion to that color.";
const NYX_LOTUS: &str = "This artifact enters tapped.\n{T}: Choose a color. Add an amount of mana of that color equal to your devotion to that color.";
const KCI: &str = "Sacrifice an artifact: Add {C}{C}.";
const SUNKEN_CITADEL: &str = "This land enters tapped. As it enters, choose a color.\n{T}: Add one mana of the chosen color.\n{T}: Add two mana of the chosen color. Spend this mana only to activate abilities of land sources.";
const UNKNOWN_EVENT_SHORES: &str = "When building your deck, choose white, blue, black, red, or green (Circle it to indicate your selection.)\n{T}: Add one mana of the chosen color.\n{1}, {T}: Add 1 mana of any color.";
const CHALICE: &str = "Multikicker {2}\nThis artifact enters with a charge counter on it for each time it was kicked.\n{T}: Add {C} for each charge counter on this artifact.";
const SOL_RING: &str = "{T}: Add {C}{C}.";
const METEOR_CRATER: &str =
    "{T}: Choose a color of a permanent you control. Add one mana of that color.";
const CORRUPTED_GRAFSTONE: &str = "This artifact enters tapped.\n{T}: Choose a color of a card in your graveyard. Add one mana of that color.";

fn charge() -> CounterType {
    CounterType::Generic("charge".to_string())
}

fn cost(shards: Vec<ManaCostShard>, generic: u32) -> ManaCost {
    ManaCost::Cost { shards, generic }
}

fn add_instant(scenario: &mut GameScenario, mana_cost: ManaCost) -> ObjectId {
    scenario.with_library_top(P0, &["Library A", "Library B", "Library C"]);
    scenario
        .add_spell_to_hand_from_oracle(P0, "Test Instant", true, "Draw a card.")
        .with_mana_cost(mana_cost)
        .id()
}

/// Priority facts for `spell`: (CastSpell offered, auto-pass recommended).
fn priority_facts(runner: &GameRunner, spell: ObjectId) -> (bool, bool) {
    let state = runner.state();
    let offered = legal_actions(state).iter().any(
        |action| matches!(action, GameAction::CastSpell { object_id, .. } if *object_id == spell),
    );
    (
        offered,
        auto_pass_recommended(state, &flat_priority_actions(state)),
    )
}

/// The same facts in a priority window on the opponent's turn, which is where
/// the report hit it (holding up an instant). Restores P0 as active player.
fn opponent_turn_priority_facts(runner: &mut GameRunner, spell: ObjectId) -> (bool, bool) {
    set_priority(runner, P1);
    let facts = priority_facts(runner, spell);
    set_priority(runner, P0);
    facts
}

fn set_priority(runner: &mut GameRunner, active: engine::types::player::PlayerId) {
    let state = runner.state_mut();
    state.active_player = active;
    state.priority_player = P0;
    state.waiting_for = WaitingFor::Priority { player: P0 };
}

fn produced_of(runner: &GameRunner, source: ObjectId, index: usize) -> ManaProduction {
    match &*runner.state().objects[&source].abilities[index].effect {
        Effect::Mana { produced, .. } => produced.clone(),
        other => panic!("ability {index} must be a mana ability, got {other:?}"),
    }
}

fn spell_zone(runner: &GameRunner, spell: ObjectId) -> Zone {
    runner.state().objects[&spell].zone
}

/// Reach guard shared by every Astral leg: the printed ability is a pure
/// chosen-color production with no color chosen and no linked chooser.
fn assert_astral_reach(runner: &GameRunner, astral: ObjectId) {
    assert!(
        matches!(
            produced_of(runner, astral, 0),
            ManaProduction::ChosenColor {
                fixed_alternative: None,
                ..
            }
        ),
        "Astral Cornucopia must parse to a pure chosen-color production"
    );
    let obj = &runner.state().objects[&astral];
    assert_eq!(
        obj.chosen_color(),
        None,
        "no color chosen before activation"
    );
    assert!(
        obj.replacement_definitions
            .iter_unchecked()
            .filter_map(|replacement| replacement.execute.as_deref())
            .all(|execute| !matches!(&*execute.effect, Effect::Choose { .. })),
        "Astral Cornucopia has no linked color chooser"
    );
}

/// CR 106.1 + CR 607.2d + CR 117.1d: Astral Cornucopia with N charge counters
/// pays for spells up to N mana of one color: offered on both turns, auto-pass
/// holds, and the auto-paid cast resolves with the excess left in the pool in
/// the color the cost needed.
#[test]
fn astral_cornucopia_pays_for_spells_at_each_counter_count() {
    let legs: [(u32, ManaCost, ManaType); 3] = [
        (1, cost(vec![ManaCostShard::Red], 0), ManaType::Red),
        (3, cost(vec![ManaCostShard::Red], 2), ManaType::Red),
        // The reported shape: Fuel for the Cause is {2}{U}{U}.
        (
            6,
            cost(vec![ManaCostShard::Blue, ManaCostShard::Blue], 2),
            ManaType::Blue,
        ),
    ];
    for (counters, mana_cost, color) in legs {
        let total = mana_cost.mana_value();
        let mut scenario = GameScenario::new();
        scenario.at_phase(Phase::PreCombatMain);
        let astral = scenario
            .add_artifact_from_oracle(P0, "Astral Cornucopia", ASTRAL)
            .id();
        scenario.with_counter(astral, charge(), counters);
        let spell = add_instant(&mut scenario, mana_cost);
        let mut runner = scenario.build();
        assert_astral_reach(&runner, astral);

        assert_eq!(
            priority_facts(&runner, spell),
            (true, false),
            "{counters} counters: own turn offers the cast and holds priority"
        );
        assert_eq!(
            opponent_turn_priority_facts(&mut runner, spell),
            (true, false),
            "{counters} counters: opponent's turn offers the cast and holds priority"
        );

        runner.cast(spell).resolve();
        assert_eq!(
            spell_zone(&runner, spell),
            Zone::Graveyard,
            "{counters} counters: the auto-paid cast resolves"
        );
        assert!(
            runner.state().objects[&astral].tapped,
            "Astral paid the cost"
        );
        let pool = &runner.state().players[0].mana_pool;
        assert_eq!(
            pool.count_color(color),
            (counters - total) as usize,
            "{counters} counters: the excess floats in the color the cost needed"
        );
        assert_eq!(
            pool.total(),
            (counters - total) as usize,
            "one activation, one color"
        );
    }
}

/// CR 106.1 + CR 107.3 + CR 601.2f: the largest X offered counts Astral
/// Cornucopia's full output. Four counters pay {X}{R} with X up to 3.
#[test]
fn astral_cornucopia_counts_its_full_output_toward_x() {
    let mut scenario = GameScenario::new();
    scenario.at_phase(Phase::PreCombatMain);
    let astral = scenario
        .add_artifact_from_oracle(P0, "Astral Cornucopia", ASTRAL)
        .id();
    scenario.with_counter(astral, charge(), 4);
    scenario.with_library_top(P0, &["Library A", "Library B", "Library C", "Library D"]);
    let spell = scenario
        .add_spell_to_hand_from_oracle(P0, "X Draw", true, "Draw X cards.")
        .with_mana_cost(cost(vec![ManaCostShard::X, ManaCostShard::Red], 0))
        .id();
    let mut runner = scenario.build();
    assert_astral_reach(&runner, astral);

    let card_id = runner.state().objects[&spell].card_id;
    runner
        .act(GameAction::CastSpell {
            object_id: spell,
            card_id,
            targets: vec![],
            payment_mode: CastPaymentMode::Auto,
        })
        .expect("the X spell can be cast");
    match &runner.state().waiting_for {
        WaitingFor::ChooseXValue { max, .. } => assert_eq!(*max, 3),
        other => panic!("expected the X choice, got {other:?}"),
    }
    runner
        .act(GameAction::ChooseX { value: 3 })
        .expect("the offered maximum X is payable");
}

/// CR 106.1: Astral Cornucopia with fewer counters than the cost stays
/// uncastable and auto-pass still passes (the fix does not over-count).
#[test]
fn astral_cornucopia_does_not_cover_more_than_its_counters() {
    let mut scenario = GameScenario::new();
    scenario.at_phase(Phase::PreCombatMain);
    let astral = scenario
        .add_artifact_from_oracle(P0, "Astral Cornucopia", ASTRAL)
        .id();
    scenario.with_counter(astral, charge(), 3);
    let spell = add_instant(&mut scenario, cost(vec![ManaCostShard::Red], 3));
    let mut runner = scenario.build();
    assert_astral_reach(&runner, astral);

    assert_eq!(priority_facts(&runner, spell), (false, true));
    assert_eq!(
        opponent_turn_priority_facts(&mut runner, spell),
        (false, true)
    );
}

/// CR 106.1 + CR 700.5 + CR 607.2d: Nykthos's amount depends on the color
/// chosen during activation, so each color is planned with its own devotion.
/// Devotion 3 to green pays {G}{G}{G}; devotion 1 to blue cannot pay {U}{U}.
#[test]
fn nykthos_plans_each_color_with_its_own_devotion() {
    let build = |mana_cost: ManaCost| {
        let mut scenario = GameScenario::new();
        scenario.at_phase(Phase::PreCombatMain);
        let nykthos = scenario
            .add_land_from_oracle(P0, "Nykthos, Shrine to Nyx", NYKTHOS)
            .id();
        scenario.add_basic_land(P0, ManaColor::White);
        scenario.add_basic_land(P0, ManaColor::White);
        scenario
            .add_creature(P0, "Green Devotee", 1, 1)
            .with_mana_cost(cost(
                vec![
                    ManaCostShard::Green,
                    ManaCostShard::Green,
                    ManaCostShard::Green,
                ],
                0,
            ));
        scenario
            .add_creature(P0, "Blue Devotee", 1, 1)
            .with_mana_cost(cost(vec![ManaCostShard::Blue], 0));
        let spell = add_instant(&mut scenario, mana_cost);
        (scenario.build(), nykthos, spell)
    };

    let (mut runner, nykthos, spell) = build(cost(
        vec![
            ManaCostShard::Green,
            ManaCostShard::Green,
            ManaCostShard::Green,
        ],
        0,
    ));
    assert!(
        matches!(
            produced_of(&runner, nykthos, 1),
            ManaProduction::ChosenColor {
                fixed_alternative: None,
                ..
            }
        ),
        "reach: Nykthos's second ability is a pure chosen-color production"
    );
    assert_eq!(priority_facts(&runner, spell), (true, false));
    assert_eq!(
        opponent_turn_priority_facts(&mut runner, spell),
        (true, false)
    );
    runner.cast(spell).resolve();
    assert_eq!(spell_zone(&runner, spell), Zone::Graveyard);
    assert!(runner.state().objects[&nykthos].tapped);

    let (mut runner, _, spell) = build(cost(vec![ManaCostShard::Blue, ManaCostShard::Blue], 0));
    assert_eq!(
        priority_facts(&runner, spell),
        (false, true),
        "devotion 1 to blue cannot pay {{U}}{{U}}"
    );
    assert_eq!(
        opponent_turn_priority_facts(&mut runner, spell),
        (false, true)
    );
}

/// CR 106.1 + CR 700.5 + CR 607.2d + CR 601.2g: Nyx Lotus has no mana in its
/// activation cost. Alone it is planned through auto-tap; next to a manual
/// sacrifice mana ability (Krark-Clan Ironworks) castability is planned from
/// activatable mana profiles. Each color carries its own devotion either way.
#[test]
fn nyx_lotus_plans_each_color_with_its_own_devotion() {
    let build = |mana_cost: ManaCost, with_ironworks: bool| {
        let mut scenario = GameScenario::new();
        scenario.at_phase(Phase::PreCombatMain);
        let lotus = scenario
            .add_artifact_from_oracle(P0, "Nyx Lotus", NYX_LOTUS)
            .id();
        if with_ironworks {
            scenario.add_artifact_from_oracle(P0, "Krark-Clan Ironworks", KCI);
            scenario.add_artifact_from_oracle(P0, "Spare Artifact", "");
        }
        scenario
            .add_creature(P0, "Green Devotee", 1, 1)
            .with_mana_cost(cost(
                vec![
                    ManaCostShard::Green,
                    ManaCostShard::Green,
                    ManaCostShard::Green,
                ],
                0,
            ));
        scenario
            .add_creature(P0, "Blue Devotee", 1, 1)
            .with_mana_cost(cost(vec![ManaCostShard::Blue], 0));
        let spell = add_instant(&mut scenario, mana_cost);
        (scenario.build(), lotus, spell)
    };
    let green3_plus_2 = || {
        cost(
            vec![
                ManaCostShard::Green,
                ManaCostShard::Green,
                ManaCostShard::Green,
            ],
            2,
        )
    };
    let blue2_plus_2 = || cost(vec![ManaCostShard::Blue, ManaCostShard::Blue], 2);

    // Auto-tap path.
    let (mut runner, lotus, spell) = build(cost(vec![ManaCostShard::Green], 2), false);
    assert!(matches!(
        produced_of(&runner, lotus, 0),
        ManaProduction::ChosenColor {
            fixed_alternative: None,
            ..
        }
    ));
    assert_eq!(priority_facts(&runner, spell), (true, false));
    assert_eq!(
        opponent_turn_priority_facts(&mut runner, spell),
        (true, false)
    );
    runner.cast(spell).resolve();
    assert_eq!(spell_zone(&runner, spell), Zone::Graveyard);
    assert!(runner.state().objects[&lotus].tapped);

    let (runner, _, spell) = build(cost(vec![ManaCostShard::Blue], 2), false);
    assert_eq!(
        priority_facts(&runner, spell),
        (false, true),
        "devotion 1 to blue cannot pay {{2}}{{U}}"
    );

    // Reach guard for the profile path: without Ironworks, five mana is out
    // of reach, so the castable leg below needs the profile combination.
    let (runner, _, spell) = build(green3_plus_2(), false);
    assert!(!priority_facts(&runner, spell).0);

    // Profile path (Ironworks' sacrifice is a manual mana choice).
    let (mut runner, _, spell) = build(green3_plus_2(), true);
    assert!(
        priority_facts(&runner, spell).0,
        "GGG from Nyx Lotus plus CC from Ironworks pays {{2}}{{G}}{{G}}{{G}}"
    );
    assert!(opponent_turn_priority_facts(&mut runner, spell).0);

    let (runner, _, spell) = build(blue2_plus_2(), true);
    assert!(
        !priority_facts(&runner, spell).0,
        "devotion 1 to blue cannot pay {{U}}{{U}} even with Ironworks"
    );
}

/// CR 106.5 + CR 607.2d fail-closed control for the chooser traversal: a
/// persisted color chooser nested inside an inline branch carrier
/// (`ChooseOneOf`) is still a linked chooser, so the unset source adds no mana.
#[test]
fn linked_color_chooser_inside_a_branch_carrier_counts_as_linked() {
    let mut scenario = GameScenario::new();
    scenario.at_phase(Phase::PreCombatMain);
    let astral = scenario
        .add_artifact_from_oracle(P0, "Astral Cornucopia", ASTRAL)
        .id();
    scenario.with_counter(astral, charge(), 3);
    let spell = add_instant(&mut scenario, cost(vec![ManaCostShard::Red], 0));
    let mut runner = scenario.build();
    assert!(
        priority_facts(&runner, spell).0,
        "reach: without a linked chooser the source pays {{R}}"
    );

    let nested_chooser = AbilityDefinition::new(
        AbilityKind::Spell,
        Effect::ChooseOneOf {
            chooser: PlayerFilter::Controller,
            branches: vec![AbilityDefinition::new(
                AbilityKind::Spell,
                Effect::Choose {
                    choice_type: ChoiceType::color(),
                    persist: true,
                    selection: TargetSelectionMode::Chosen,
                },
            )],
        },
    );
    let replacement = ReplacementDefinition::new(ReplacementEvent::Moved).execute(nested_chooser);
    {
        let obj = runner.state_mut().objects.get_mut(&astral).unwrap();
        // Both the live list and its layer base, so a layer pass keeps it.
        std::sync::Arc::make_mut(&mut obj.base_replacement_definitions).push(replacement.clone());
        obj.replacement_definitions.push(replacement);
    }
    assert_eq!(
        priority_facts(&runner, spell),
        (false, true),
        "a nested linked chooser left unset produces no mana (CR 106.5)"
    );
}

/// Reviewer probe board (#8443 r1, R1): Nyx Lotus's per-color amounts must
/// still let surplus mana of the chosen color pay generic. Devotion G3 / U1;
/// Krark-Clan Ironworks makes the payment a manual mana choice, so the
/// activatable-profile path decides. 3G + CC pays {2}{G}{G} with one left
/// over (CR 700.5, CR 106.4, CR 601.2g).
#[test]
fn nyx_lotus_surplus_color_pays_generic_alongside_ironworks() {
    const HARMONIZE: &str = "Draw three cards.";
    let build = |with_ironworks: bool| {
        let mut scenario = GameScenario::new();
        scenario.at_phase(Phase::PreCombatMain);
        scenario.with_library_top(P0, &["Library A", "Library B", "Library C", "Library D"]);
        let lotus = scenario
            .add_artifact_from_oracle(
                P0,
                "Nyx Lotus",
                "Nyx Lotus enters tapped.\n{T}: Choose a color. Add an amount of mana of that color equal to your devotion to that color.",
            )
            .id();
        scenario
            .add_creature(P0, "Leatherback Baloth", 4, 5)
            .with_mana_cost(cost(
                vec![
                    ManaCostShard::Green,
                    ManaCostShard::Green,
                    ManaCostShard::Green,
                ],
                0,
            ));
        scenario
            .add_creature(P0, "Merfolk of the Pearl Trident", 1, 1)
            .with_mana_cost(cost(vec![ManaCostShard::Blue], 0));
        let mut ironworks = None;
        let mut ornithopter = None;
        if with_ironworks {
            ironworks = Some(
                scenario
                    .add_artifact_from_oracle(P0, "Krark-Clan Ironworks", KCI)
                    .id(),
            );
            ornithopter = Some(
                scenario
                    .add_creature(P0, "Ornithopter", 0, 2)
                    .as_artifact_creature()
                    .from_oracle_text("Flying")
                    .with_mana_cost(cost(vec![], 0))
                    .id(),
            );
        }
        let harmonize = scenario
            .add_spell_to_hand_from_oracle(P0, "Harmonize", false, HARMONIZE)
            .with_mana_cost(cost(vec![ManaCostShard::Green, ManaCostShard::Green], 2))
            .id();
        (scenario.build(), lotus, ironworks, ornithopter, harmonize)
    };

    // Reach: the Lotus ability is a pure chosen-color production with no
    // recorded color.
    let (runner, lotus, _, _, harmonize) = build(true);
    assert!(matches!(
        produced_of(&runner, lotus, 0),
        ManaProduction::ChosenColor {
            fixed_alternative: None,
            ..
        }
    ));
    assert_eq!(runner.state().objects[&lotus].chosen_color(), None);

    assert_eq!(
        priority_facts(&runner, harmonize),
        (true, false),
        "GGG from Nyx Lotus plus CC from Ironworks pays {{2}}{{G}}{{G}}"
    );

    // Control: without Ironworks, three mana cannot pay four.
    let (runner, _, _, _, harmonize) = build(false);
    assert!(!priority_facts(&runner, harmonize).0);

    // Control: tap the Lotus for green, sacrifice Ornithopter to Ironworks,
    // and the cast resolves with one green left over.
    let (mut runner, lotus, ironworks, ornithopter, harmonize) = build(true);
    runner
        .act(GameAction::ActivateAbility {
            source_id: lotus,
            ability_index: 0,
        })
        .expect("Nyx Lotus activates");
    runner
        .act(GameAction::ChooseManaColor {
            choice: ManaChoice::SingleColor(ManaType::Green),
            count: 1,
        })
        .expect("choose green");
    assert_eq!(
        runner.state().players[0]
            .mana_pool
            .count_color(ManaType::Green),
        3,
        "devotion to green is 3"
    );
    let _ = ironworks;
    let card_id = runner.state().objects[&harmonize].card_id;
    runner
        .act(GameAction::CastSpell {
            object_id: harmonize,
            card_id,
            targets: vec![],
            payment_mode: CastPaymentMode::Manual,
        })
        .expect("Harmonize is cast with manual payment");
    assert!(matches!(
        runner.state().waiting_for,
        WaitingFor::ManaPayment { .. }
    ));
    runner
        .act(GameAction::ActivateAbility {
            source_id: ironworks.unwrap(),
            ability_index: 0,
        })
        .expect("Ironworks activates during payment");
    runner
        .act(GameAction::SelectCards {
            cards: vec![ornithopter.unwrap()],
        })
        .expect("sacrifice Ornithopter");
    runner
        .act(GameAction::PassPriority)
        .expect("finalize the payment");
    assert_eq!(
        runner.state().players[0].mana_pool.total(),
        1,
        "GGG + CC paid {{2}}{{G}}{{G}} with one mana unspent"
    );
    runner.resolve_top();
    assert_eq!(spell_zone(&runner, harmonize), Zone::Graveyard);
    assert_eq!(
        runner.state().players[0].hand.len(),
        3,
        "Harmonize drew three cards"
    );
}

/// Where the Island sits relative to the Lotus. Profile grouping is
/// ObjectId-sorted, so this controls which source is tried first.
#[derive(Clone, Copy, PartialEq, Eq)]
enum IslandOrder {
    BeforeLotus,
    AfterLotus,
}

/// Shared Nyx Lotus board (devotion G3 / U1) for the per-color accounting
/// boards, with an optional Island at the given order.
struct LotusBoard {
    runner: GameRunner,
    lotus: ObjectId,
    island: Option<ObjectId>,
    spell: ObjectId,
}

fn lotus_board(
    island_order: Option<IslandOrder>,
    with_ironworks: bool,
    spell_name: &str,
    spell_oracle: &str,
    spell_cost: ManaCost,
    instant: bool,
) -> LotusBoard {
    let mut scenario = GameScenario::new();
    scenario.at_phase(Phase::PreCombatMain);
    scenario.with_library_top(P0, &["Library A", "Library B", "Library C", "Library D"]);
    let mut island = None;
    if island_order == Some(IslandOrder::BeforeLotus) {
        island = Some(scenario.add_basic_land(P0, ManaColor::Blue));
    }
    let lotus = scenario
        .add_artifact_from_oracle(P0, "Nyx Lotus", NYX_LOTUS)
        .id();
    if island_order == Some(IslandOrder::AfterLotus) {
        island = Some(scenario.add_basic_land(P0, ManaColor::Blue));
    }
    scenario
        .add_creature(P0, "Leatherback Baloth", 4, 5)
        .with_mana_cost(cost(
            vec![
                ManaCostShard::Green,
                ManaCostShard::Green,
                ManaCostShard::Green,
            ],
            0,
        ));
    scenario
        .add_creature(P0, "Merfolk of the Pearl Trident", 1, 1)
        .with_mana_cost(cost(vec![ManaCostShard::Blue], 0));
    if with_ironworks {
        scenario.add_artifact_from_oracle(P0, "Krark-Clan Ironworks", KCI);
        scenario
            .add_creature(P0, "Ornithopter", 0, 2)
            .as_artifact_creature()
            .from_oracle_text("Flying");
    }
    let spell = scenario
        .add_spell_to_hand_from_oracle(P0, spell_name, instant, spell_oracle)
        .with_mana_cost(spell_cost)
        .id();
    LotusBoard {
        runner: scenario.build(),
        lotus,
        island,
        spell,
    }
}

/// Reviewer probe board (#8443 r2, R1a): green from Nyx Lotus covers {G} and
/// its two other green pay generic, while the {U} comes from an Island tried
/// after it. GGG + U + CC pays {3}{G}{U} with one to spare (CR 106.4,
/// CR 601.2g, CR 700.5).
#[test]
fn nyx_lotus_covers_part_of_the_colored_shards_and_leaves_the_rest() {
    const URBAN_EVOLUTION: &str = "Draw three cards. You may play an additional land this turn.";
    let urban_evolution = || cost(vec![ManaCostShard::Green, ManaCostShard::Blue], 3);

    let board = lotus_board(
        Some(IslandOrder::AfterLotus),
        true,
        "Urban Evolution",
        URBAN_EVOLUTION,
        urban_evolution(),
        false,
    );
    let island = board.island.unwrap();
    assert!(
        board.lotus < island,
        "reach: the Lotus is grouped before the Island"
    );
    assert!(matches!(
        produced_of(&board.runner, board.lotus, 0),
        ManaProduction::ChosenColor {
            fixed_alternative: None,
            ..
        }
    ));
    assert_eq!(
        board.runner.state().objects[&board.lotus].chosen_color(),
        None
    );
    assert_eq!(
        priority_facts(&board.runner, board.spell),
        (true, false),
        "GGG + U + CC pays {{3}}{{G}}{{U}}"
    );

    // Control: without Ironworks, four mana cannot pay five.
    let board = lotus_board(
        Some(IslandOrder::AfterLotus),
        false,
        "Urban Evolution",
        URBAN_EVOLUTION,
        urban_evolution(),
        false,
    );
    assert!(!priority_facts(&board.runner, board.spell).0);

    // Control: Island created first.
    let board = lotus_board(
        Some(IslandOrder::BeforeLotus),
        true,
        "Urban Evolution",
        URBAN_EVOLUTION,
        urban_evolution(),
        false,
    );
    assert!(board.island.unwrap() < board.lotus);
    assert!(priority_facts(&board.runner, board.spell).0);
}

/// Reviewer probe board (#8443 r2, R1b): choosing blue from Nyx Lotus for a
/// {U} adds one mana, not green's three. Blue gives U + R (Ogre); green gives
/// GGG + R with no U. Treasonous Ogre's "Pay 3 life" works once at 4 life
/// (CR 119.4). Planning must credit the chosen color's amount (CR 700.5,
/// CR 601.2g).
#[test]
fn nyx_lotus_generic_capacity_follows_the_chosen_color() {
    const OGRE: &str = "Dethrone\nPay 3 life: Add {R}.";
    const INSPIRATION: &str = "Target player draws two cards.";
    struct Board {
        runner: GameRunner,
        lotus: ObjectId,
        ogre: ObjectId,
        inspiration: ObjectId,
        divination: ObjectId,
        blue_two: ObjectId,
    }
    let build = |floating_blue: bool| {
        let mut scenario = GameScenario::new();
        scenario.at_phase(Phase::PreCombatMain);
        scenario.with_life(P0, 4);
        let lotus = scenario
            .add_artifact_from_oracle(P0, "Nyx Lotus", NYX_LOTUS)
            .id();
        scenario
            .add_creature(P0, "Leatherback Baloth", 4, 5)
            .with_mana_cost(cost(
                vec![
                    ManaCostShard::Green,
                    ManaCostShard::Green,
                    ManaCostShard::Green,
                ],
                0,
            ));
        scenario
            .add_creature(P0, "Merfolk of the Pearl Trident", 1, 1)
            .with_mana_cost(cost(vec![ManaCostShard::Blue], 0));
        let ogre = scenario
            .add_creature(P0, "Treasonous Ogre", 2, 3)
            .from_oracle_text(OGRE)
            .with_mana_cost(cost(vec![ManaCostShard::Red], 3))
            .id();
        if floating_blue {
            scenario.with_mana_pool(
                P0,
                vec![ManaUnit::new(ManaType::Blue, ObjectId(0), false, vec![])],
            );
        }
        let inspiration = scenario
            .add_spell_to_hand_from_oracle(P0, "Inspiration", true, INSPIRATION)
            .with_mana_cost(cost(vec![ManaCostShard::Blue], 3))
            .id();
        let divination = scenario
            .add_spell_to_hand_from_oracle(P0, "Divination", false, "Draw two cards.")
            .with_mana_cost(cost(vec![ManaCostShard::Blue], 2))
            .id();
        let blue_two = scenario
            .add_spell_to_hand_from_oracle(P0, "Blue Two", true, "Draw a card.")
            .with_mana_cost(cost(vec![ManaCostShard::Blue], 1))
            .id();
        Board {
            runner: scenario.build(),
            lotus,
            ogre,
            inspiration,
            divination,
            blue_two,
        }
    };
    let offered = |board: &Board, spell: ObjectId| priority_facts(&board.runner, spell).0;

    let board = build(false);
    assert!(matches!(
        produced_of(&board.runner, board.lotus, 0),
        ManaProduction::ChosenColor {
            fixed_alternative: None,
            ..
        }
    ));
    for (color, devotion) in [
        (ManaColor::Green, 3),
        (ManaColor::Blue, 1),
        (ManaColor::Red, 1),
    ] {
        assert_eq!(
            engine::game::devotion::count_devotion(board.runner.state(), P0, &[color]),
            devotion,
            "devotion to {color:?}"
        );
    }
    // Reach: an untargeted {1}{U} is payable only by Lotus blue plus the
    // Ogre's manual pay-life mana, so the activatable-profile path decides.
    assert!(
        offered(&board, board.blue_two),
        "Lotus U + Ogre R pays {{1}}{{U}}"
    );
    // The board verbatim: {3}{U} is out of reach.
    assert!(!offered(&board, board.inspiration));
    // Discriminating twin: an untargeted {2}{U}. Blue from the Lotus credits
    // one mana, so U + R is two of three; crediting green's three would offer it.
    assert!(
        !offered(&board, board.divination),
        "blue from the Lotus plus the Ogre's R is two mana, not four"
    );

    // Control: the Ogre activates once at 4 life and not again at 1.
    let mut board = build(false);
    board
        .runner
        .act(GameAction::ActivateAbility {
            source_id: board.ogre,
            ability_index: 0,
        })
        .expect("the Ogre activates at 4 life");
    assert_eq!(board.runner.state().players[0].life, 1);
    assert!(
        board
            .runner
            .act(GameAction::ActivateAbility {
                source_id: board.ogre,
                ability_index: 0,
            })
            .is_err(),
        "CR 119.4: 3 life cannot be paid at 1"
    );

    // Positive witness: one floating blue pays {U}; GGG + R covers {3}.
    let board = build(true);
    assert!(offered(&board, board.inspiration));
}

/// CR 106.1 + CR 700.5 + CR 601.2g: generic mana from Nyx Lotus uses the
/// color that adds the most (green, devotion 3), so {3} is payable by the
/// Lotus alone and the cast resolves.
#[test]
fn nyx_lotus_pays_generic_with_its_best_color() {
    let mut scenario = GameScenario::new();
    scenario.at_phase(Phase::PreCombatMain);
    let lotus = scenario
        .add_artifact_from_oracle(P0, "Nyx Lotus", NYX_LOTUS)
        .id();
    scenario
        .add_creature(P0, "Leatherback Baloth", 4, 5)
        .with_mana_cost(cost(
            vec![
                ManaCostShard::Green,
                ManaCostShard::Green,
                ManaCostShard::Green,
            ],
            0,
        ));
    scenario
        .add_creature(P0, "Merfolk of the Pearl Trident", 1, 1)
        .with_mana_cost(cost(vec![ManaCostShard::Blue], 0));
    let spell = add_instant(&mut scenario, cost(vec![], 3));
    let mut runner = scenario.build();
    assert!(matches!(
        produced_of(&runner, lotus, 0),
        ManaProduction::ChosenColor {
            fixed_alternative: None,
            ..
        }
    ));
    assert_eq!(priority_facts(&runner, spell), (true, false));
    assert_eq!(
        opponent_turn_priority_facts(&mut runner, spell),
        (true, false)
    );
    runner.cast(spell).resolve();
    assert_eq!(spell_zone(&runner, spell), Zone::Graveyard);
    assert!(runner.state().objects[&lotus].tapped);
}

/// Reviewer probe board (#8443 r2, R2a, engine-shape fixture): an ability on
/// Astral Cornucopia that grants a color chooser to another object (CR 611.2)
/// is that object's chooser, not a linked chooser for Cornucopia.
#[test]
fn chooser_granted_to_another_object_is_not_linked() {
    let mut scenario = GameScenario::new();
    scenario.at_phase(Phase::PreCombatMain);
    scenario.with_library_top(P0, &["Library A", "Library B", "Library C"]);
    let astral = scenario
        .add_artifact_from_oracle(P0, "Astral Cornucopia", ASTRAL)
        .id();
    scenario.with_counter(astral, charge(), 4);
    let baloth = scenario
        .add_creature(P0, "Leatherback Baloth", 4, 5)
        .with_mana_cost(cost(
            vec![
                ManaCostShard::Green,
                ManaCostShard::Green,
                ManaCostShard::Green,
            ],
            0,
        ))
        .id();
    let harmonize = scenario
        .add_spell_to_hand_from_oracle(P0, "Harmonize", false, "Draw three cards.")
        .with_mana_cost(cost(vec![ManaCostShard::Green, ManaCostShard::Green], 2))
        .id();
    let mut runner = scenario.build();
    assert!(
        priority_facts(&runner, harmonize).0,
        "reach: plain Cornucopia pays Harmonize"
    );

    let grant = AbilityDefinition::new(
        AbilityKind::Activated,
        Effect::GenericEffect {
            static_abilities: vec![StaticDefinition::continuous()
                .affected(TargetFilter::SpecificObject { id: baloth })
                .modifications(vec![ContinuousModification::GrantAbility {
                    definition: Box::new(AbilityDefinition::new(
                        AbilityKind::Spell,
                        Effect::Choose {
                            choice_type: ChoiceType::color(),
                            persist: true,
                            selection: TargetSelectionMode::Chosen,
                        },
                    )),
                }])],
            duration: Some(Duration::UntilEndOfTurn),
            target: None,
            end_cost: None,
        },
    )
    .cost(AbilityCost::Mana {
        cost: cost(vec![], 9),
    });
    {
        let obj = runner.state_mut().objects.get_mut(&astral).unwrap();
        std::sync::Arc::make_mut(&mut obj.base_abilities).push(grant.clone());
        std::sync::Arc::make_mut(&mut obj.abilities).push(grant);
    }
    assert!(
        matches!(
            &*runner.state().objects[&astral].abilities[1].effect,
            Effect::GenericEffect { static_abilities, .. }
                if matches!(
                    &static_abilities[0].affected,
                    Some(TargetFilter::SpecificObject { id }) if *id == baloth
                )
        ),
        "reach: the grant targets the Baloth"
    );
    assert!(
        priority_facts(&runner, harmonize).0,
        "a chooser granted to the Baloth does not make Cornucopia's color linked"
    );
}

/// CR 107.3 + CR 601.2f + CR 700.5 (fail closed, #8443 r3 ruling): the {X}
/// ceiling sums mana capacities with no per-color accounting, so a source
/// that chooses its color on activation and adds a different amount per color
/// is not counted. Nyx Lotus (G3 / U1) must spend its blue on a {U}, so for
/// Mind Spring {X}{U}{U} with one Island the ceiling is 0 either way.
#[test]
fn x_ceiling_leaves_out_a_color_dependent_activation_amount() {
    const MIND_SPRING: &str = "Draw X cards.";
    let build = |floating_blue: u32| {
        let mut scenario = GameScenario::new();
        scenario.at_phase(Phase::PreCombatMain);
        scenario.with_library_top(P0, &["Library A", "Library B", "Library C", "Library D"]);
        let lotus = scenario
            .add_artifact_from_oracle(P0, "Nyx Lotus", NYX_LOTUS)
            .id();
        scenario
            .add_creature(P0, "Leatherback Baloth", 4, 5)
            .with_mana_cost(cost(
                vec![
                    ManaCostShard::Green,
                    ManaCostShard::Green,
                    ManaCostShard::Green,
                ],
                0,
            ));
        scenario
            .add_creature(P0, "Merfolk of the Pearl Trident", 1, 1)
            .with_mana_cost(cost(vec![ManaCostShard::Blue], 0));
        scenario.add_basic_land(P0, ManaColor::Blue);
        if floating_blue > 0 {
            scenario.with_mana_pool(
                P0,
                (0..floating_blue)
                    .map(|_| ManaUnit::new(ManaType::Blue, ObjectId(0), false, vec![]))
                    .collect(),
            );
        }
        let spell = scenario
            .add_spell_to_hand_from_oracle(P0, "Mind Spring", false, MIND_SPRING)
            .with_mana_cost(cost(
                vec![ManaCostShard::X, ManaCostShard::Blue, ManaCostShard::Blue],
                0,
            ))
            .id();
        (scenario.build(), lotus, spell)
    };
    let x_ceiling = |runner: &mut GameRunner, spell: ObjectId| {
        let card_id = runner.state().objects[&spell].card_id;
        runner
            .act(GameAction::CastSpell {
                object_id: spell,
                card_id,
                targets: vec![],
                payment_mode: CastPaymentMode::Auto,
            })
            .expect("Mind Spring can be cast");
        match &runner.state().waiting_for {
            WaitingFor::ChooseXValue { max, .. } => *max,
            other => panic!("expected the X choice, got {other:?}"),
        }
    };

    let (mut runner, lotus, spell) = build(0);
    assert!(matches!(
        produced_of(&runner, lotus, 0),
        ManaProduction::ChosenColor {
            fixed_alternative: None,
            ..
        }
    ));
    assert_eq!(runner.state().objects[&lotus].chosen_color(), None);
    for (color, devotion) in [(ManaColor::Green, 3), (ManaColor::Blue, 1)] {
        assert_eq!(
            engine::game::devotion::count_devotion(runner.state(), P0, &[color]),
            devotion
        );
    }
    assert_eq!(x_ceiling(&mut runner, spell), 0);
    runner
        .act(GameAction::ChooseX { value: 0 })
        .expect("X = 0 is payable: blue from the Lotus and the Island");
    assert!(
        runner.state().stack.iter().any(|entry| entry.id == spell),
        "Mind Spring is on the stack, paid"
    );
    assert!(
        runner.state().objects[&lotus].tapped,
        "the Lotus paid {{U}}"
    );
    assert_eq!(
        runner.state().objects[&lotus].zone,
        Zone::Battlefield,
        "reach: the Lotus is still in play"
    );

    // Control: with UU floating the Island alone counts toward X.
    let (mut runner, _, spell) = build(2);
    assert_eq!(x_ceiling(&mut runner, spell), 1);
}

/// CR 607.2d + CR 603.12 + CR 106.5 (fail closed, #8443 r3 ruling): a persisted
/// color chooser in this object's own reflexive "when you do" ability still
/// writes the color to this object, so the unset source is linked and adds no
/// mana until the choice is made. Once it is made through real actions, the
/// color persists on the object and supplies that color. Engine-shape fixture.
#[test]
fn reflexive_color_chooser_on_the_same_object_counts_as_linked() {
    let mut scenario = GameScenario::new();
    scenario.at_phase(Phase::PreCombatMain);
    scenario.with_library_top(P0, &["Library A", "Library B", "Library C", "Library D"]);
    let astral = scenario
        .add_artifact_from_oracle(P0, "Astral Cornucopia", ASTRAL)
        .id();
    scenario.with_counter(astral, charge(), 4);
    scenario
        .add_creature(P0, "Leatherback Baloth", 4, 5)
        .with_mana_cost(cost(
            vec![
                ManaCostShard::Green,
                ManaCostShard::Green,
                ManaCostShard::Green,
            ],
            0,
        ));
    let harmonize = scenario
        .add_spell_to_hand_from_oracle(P0, "Harmonize", false, "Draw three cards.")
        .with_mana_cost(cost(vec![ManaCostShard::Green, ManaCostShard::Green], 2))
        .id();
    let mut runner = scenario.build();
    assert!(
        priority_facts(&runner, harmonize).0,
        "reach: plain Cornucopia pays Harmonize"
    );

    // "{0}: You may draw a card. When you do, choose a color." with the color
    // persisted on this object.
    let mut reflexive = AbilityDefinition::new(
        AbilityKind::Spell,
        Effect::Choose {
            choice_type: ChoiceType::color(),
            persist: true,
            selection: TargetSelectionMode::Chosen,
        },
    );
    reflexive.condition = Some(AbilityCondition::WhenYouDo);
    let mut draw = AbilityDefinition::new(
        AbilityKind::Activated,
        Effect::Draw {
            count: QuantityExpr::Fixed { value: 1 },
            target: TargetFilter::Controller,
        },
    )
    .cost(AbilityCost::Mana {
        cost: ManaCost::generic(0),
    });
    draw.optional = true;
    let draw = draw.sub_ability(reflexive);
    {
        let obj = runner.state_mut().objects.get_mut(&astral).unwrap();
        std::sync::Arc::make_mut(&mut obj.base_abilities).push(draw.clone());
        std::sync::Arc::make_mut(&mut obj.abilities).push(draw);
    }
    assert!(
        !priority_facts(&runner, harmonize).0,
        "a same-object reflexive chooser left unset gives no mana (CR 106.5)"
    );

    runner
        .activate(astral, 1)
        .accept_optional()
        .choose_option("Green")
        .resolve();
    assert_eq!(
        runner.state().objects[&astral].chosen_color(),
        Some(ManaColor::Green),
        "the reflexive choice persisted on Cornucopia"
    );
    assert!(
        priority_facts(&runner, harmonize).0,
        "the chosen green pays {{2}}{{G}}{{G}}"
    );
}

/// Reviewer probe board (#8443 r4, R4-01, engine-shape fixture): Cornucopia's
/// own "{0}: Choose a color" is linked even when the same definition (shared
/// by allocation through its copiable values) also appears inside a copy
/// effect installed on another object. The payload is pruned where it
/// occurs, never by address (CR 607.2d, CR 106.5, CR 707.2).
#[test]
fn own_chooser_shared_with_a_copy_payload_stays_linked() {
    let build = |with_copy_payload: bool| {
        let mut scenario = GameScenario::new();
        scenario.at_phase(Phase::PreCombatMain);
        scenario.with_library_top(P0, &["Library A", "Library B", "Library C", "Library D"]);
        let astral = scenario
            .add_artifact_from_oracle(P0, "Astral Cornucopia", ASTRAL)
            .id();
        scenario.with_counter(astral, charge(), 4);
        let baloth = scenario
            .add_creature(P0, "Leatherback Baloth", 4, 5)
            .with_mana_cost(cost(
                vec![
                    ManaCostShard::Green,
                    ManaCostShard::Green,
                    ManaCostShard::Green,
                ],
                0,
            ))
            .id();
        let harmonize = scenario
            .add_spell_to_hand_from_oracle(P0, "Harmonize", false, "Draw three cards.")
            .with_mana_cost(cost(vec![ManaCostShard::Green, ManaCostShard::Green], 2))
            .id();
        let mut runner = scenario.build();
        assert!(
            priority_facts(&runner, harmonize).0,
            "reach: plain Cornucopia pays Harmonize"
        );

        let chooser = AbilityDefinition::new(
            AbilityKind::Activated,
            Effect::Choose {
                choice_type: ChoiceType::color(),
                persist: true,
                selection: TargetSelectionMode::Chosen,
            },
        )
        .cost(AbilityCost::Mana {
            cost: ManaCost::generic(0),
        });
        {
            let obj = runner.state_mut().objects.get_mut(&astral).unwrap();
            std::sync::Arc::make_mut(&mut obj.base_abilities).push(chooser);
            obj.abilities = std::sync::Arc::clone(&obj.base_abilities);
        }
        if with_copy_payload {
            let values = engine::game::printed_cards::intrinsic_copiable_values(
                &runner.state().objects[&astral],
            );
            let copy_onto_baloth = AbilityDefinition::new(
                AbilityKind::Spell,
                Effect::GenericEffect {
                    static_abilities: vec![StaticDefinition::continuous()
                        .affected(TargetFilter::SpecificObject { id: baloth })
                        .modifications(vec![ContinuousModification::CopyValues {
                            values: Box::new(values),
                            display_source: Default::default(),
                            printed_ref: None,
                            token_image_ref: None,
                            token_art: None,
                        }])],
                    duration: Some(Duration::UntilEndOfTurn),
                    target: None,
                    end_cost: None,
                },
            );
            let replacement =
                ReplacementDefinition::new(ReplacementEvent::Moved).execute(copy_onto_baloth);
            let obj = runner.state_mut().objects.get_mut(&astral).unwrap();
            std::sync::Arc::make_mut(&mut obj.base_replacement_definitions)
                .push(replacement.clone());
            obj.replacement_definitions.push(replacement);
        }
        (runner, astral, harmonize)
    };

    let (runner, astral, harmonize) = build(true);
    // Reach: the copy payload shares Cornucopia's ability array, so the own
    // chooser and the payload's chooser are the same allocation.
    let obj = &runner.state().objects[&astral];
    let payload_abilities = obj
        .replacement_definitions
        .iter_unchecked()
        .find_map(
            |replacement| match replacement.execute.as_deref().map(|d| &*d.effect) {
                Some(Effect::GenericEffect {
                    static_abilities, ..
                }) => static_abilities[0]
                    .modifications
                    .iter()
                    .find_map(|modification| match modification {
                        ContinuousModification::CopyValues { values, .. } => {
                            Some(std::sync::Arc::clone(&values.abilities))
                        }
                        _ => None,
                    }),
                _ => None,
            },
        )
        .expect("the copy payload is installed");
    assert!(std::sync::Arc::ptr_eq(&payload_abilities, &obj.abilities));
    assert!(std::ptr::eq(
        &*payload_abilities[1].effect,
        &*obj.abilities[1].effect
    ));
    assert!(
        !priority_facts(&runner, harmonize).0,
        "the own chooser left unset keeps Cornucopia linked (CR 106.5)"
    );

    // Control: without the copy payload, still linked.
    let (runner, _, harmonize) = build(false);
    assert!(!priority_facts(&runner, harmonize).0);

    // Choosing green through real actions persists it and pays Harmonize.
    let (mut runner, astral, harmonize) = build(true);
    runner.activate(astral, 1).choose_option("Green").resolve();
    assert_eq!(
        runner.state().objects[&astral].chosen_color(),
        Some(ManaColor::Green)
    );
    assert!(priority_facts(&runner, harmonize).0);
}

/// CR 106.5 + CR 607.2d control: Sunken Citadel's "the chosen color" is linked
/// to its as-enters choice. Unset, it adds no mana; once a color is chosen it
/// pays that color.
#[test]
fn sunken_citadel_linked_choice_is_unchanged() {
    let mut scenario = GameScenario::new();
    scenario.at_phase(Phase::PreCombatMain);
    let citadel = scenario
        .add_land_from_oracle(P0, "Sunken Citadel", SUNKEN_CITADEL)
        .id();
    let spell = add_instant(&mut scenario, cost(vec![ManaCostShard::Green], 0));
    let mut runner = scenario.build();

    let obj = &runner.state().objects[&citadel];
    assert!(
        obj.replacement_definitions
            .iter_unchecked()
            .filter_map(|replacement| replacement.execute.as_deref())
            .any(|execute| matches!(
                &*execute.effect,
                Effect::Choose {
                    choice_type: ChoiceType::Color { .. },
                    persist: true,
                    ..
                }
            )),
        "reach: Sunken Citadel carries its linked as-enters color chooser"
    );
    assert!(matches!(
        produced_of(&runner, citadel, 0),
        ManaProduction::ChosenColor {
            fixed_alternative: None,
            ..
        }
    ));

    assert_eq!(
        priority_facts(&runner, spell),
        (false, true),
        "CR 106.5: no linked color chosen, no mana"
    );
    assert_eq!(
        opponent_turn_priority_facts(&mut runner, spell),
        (false, true)
    );

    runner
        .state_mut()
        .objects
        .get_mut(&citadel)
        .unwrap()
        .chosen_attributes
        .push(ChosenAttribute::Color(ManaColor::Green));
    assert_eq!(priority_facts(&runner, spell), (true, false));
    runner.cast(spell).resolve();
    assert_eq!(spell_zone(&runner, spell), Zone::Graveyard);
}

/// CR 607.2d fail-closed control: Unknown Event Shores' deck-building choice is
/// a persisted labeled choice of color words. It counts as linked, so with no
/// color recorded the source adds no mana rather than any color.
#[test]
fn unknown_event_shores_deck_building_choice_counts_as_linked() {
    let mut scenario = GameScenario::new();
    scenario.at_phase(Phase::PreCombatMain);
    let shores = scenario
        .add_land_from_oracle(P0, "Unknown Event Shores", UNKNOWN_EVENT_SHORES)
        .id();
    let spell = add_instant(&mut scenario, cost(vec![ManaCostShard::Green], 0));
    let mut runner = scenario.build();

    let obj = &runner.state().objects[&shores];
    assert!(
        obj.trigger_definitions
            .iter_unchecked()
            .filter_map(|entry| entry.definition.execute.as_deref())
            .any(|execute| matches!(
                &*execute.effect,
                Effect::Choose {
                    choice_type: ChoiceType::Labeled { .. },
                    persist: true,
                    ..
                }
            )),
        "reach: the deck-building choice is a persisted labeled chooser"
    );
    assert_eq!(obj.chosen_color(), None);
    assert!(matches!(
        produced_of(&runner, shores, 0),
        ManaProduction::ChosenColor {
            fixed_alternative: None,
            ..
        }
    ));

    assert_eq!(priority_facts(&runner, spell), (false, true));
    assert_eq!(
        opponent_turn_priority_facts(&mut runner, spell),
        (false, true)
    );
}

/// Fixed-color controls: amount-from-counters (Everflowing Chalice) and a
/// fixed multi-mana source (Sol Ring) keep paying.
#[test]
fn fixed_color_sources_still_pay() {
    for (name, oracle, counters, mana_cost) in [
        ("Everflowing Chalice", CHALICE, 3, cost(vec![], 3)),
        ("Sol Ring", SOL_RING, 0, cost(vec![], 2)),
    ] {
        let mut scenario = GameScenario::new();
        scenario.at_phase(Phase::PreCombatMain);
        let source = scenario.add_artifact_from_oracle(P0, name, oracle).id();
        scenario.with_counter(source, charge(), counters);
        let spell = add_instant(&mut scenario, mana_cost);
        let mut runner = scenario.build();
        assert_eq!(priority_facts(&runner, spell), (true, false), "{name}");
        assert_eq!(
            opponent_turn_priority_facts(&mut runner, spell),
            (true, false),
            "{name}"
        );
        runner.cast(spell).resolve();
        assert_eq!(spell_zone(&runner, spell), Zone::Graveyard, "{name}");
    }
}

/// CR 105.1 + CR 106.5: "Choose a color of a permanent you control / of a card
/// in your graveyard" restricts the color domain to game state the engine does
/// not model. The clause stays unsupported, so neither card is a mana source
/// that could pay any color.
#[test]
fn color_of_a_game_object_choice_stays_unsupported() {
    for (name, oracle) in [
        ("Meteor Crater", METEOR_CRATER),
        ("Corrupted Grafstone", CORRUPTED_GRAFSTONE),
    ] {
        let mut scenario = GameScenario::new();
        scenario.at_phase(Phase::PreCombatMain);
        let source = scenario.add_artifact_from_oracle(P0, name, oracle).id();
        scenario
            .add_creature(P0, "Red Permanent", 1, 1)
            .with_mana_cost(cost(vec![ManaCostShard::Red], 0));
        scenario.with_graveyard(P0, &["Graveyard Card"]);
        let spell = add_instant(&mut scenario, cost(vec![ManaCostShard::Red], 0));
        let runner = scenario.build();

        let ability = runner.state().objects[&source]
            .abilities
            .last()
            .expect("the tap ability parses")
            .clone();
        assert!(
            matches!(&*ability.effect, Effect::Unimplemented { .. }),
            "{name}: the restricted color choice must be unsupported, got {:?}",
            ability.effect
        );
        assert!(
            !priority_facts(&runner, spell).0,
            "{name}: not counted as mana, so the {{R}} spell is not offered"
        );
    }
}
