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
use engine::types::ability::{ChoiceType, ChosenAttribute, Effect, ManaProduction};
use engine::types::actions::GameAction;
use engine::types::counter::CounterType;
use engine::types::game_state::{CastPaymentMode, WaitingFor};
use engine::types::identifiers::ObjectId;
use engine::types::mana::{ManaColor, ManaCost, ManaCostShard, ManaType};
use engine::types::phase::Phase;
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
