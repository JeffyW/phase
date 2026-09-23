use engine::game::scenario::{GameRunner, GameScenario, P0, P1};
use engine::parser::oracle::parse_oracle_text;
use engine::types::ability::{ControllerRef, Effect, PlayerFilter, StaticCondition, TargetFilter};
use engine::types::identifiers::ObjectId;
use engine::types::mana::{ManaColor, ManaUnit};
use engine::types::phase::Phase;
use engine::types::statics::{CastFrequency, CostModifyMode, StaticMode};
use engine::types::triggers::TriggerMode;

const HOJO: &str = "The first activated ability you activate during your turn that targets a creature you control costs {2} less to activate.\nWhenever one or more creatures you control become the target of an activated ability, draw a card. This ability triggers only once each turn.";
const KOPALA: &str = "Spells your opponents cast that target a Merfolk you control cost {2} more to cast.\nAbilities your opponents activate that target a Merfolk you control cost {2} more to activate.";
const TEZZERET: &str =
    "The first activated ability of an artifact you activate each turn costs {2} less to activate.";
const FERVENT: &str = "First strike, haste\nWhenever Fervent Champion attacks, another target attacking Knight you control gets +1/+0 until end of turn.\nEquip abilities you activate that target Fervent Champion cost {3} less to activate.";
const RAFT: &str = "{2}, {T}: Tap target creature. This ability costs {1} less to activate if it targets a creature with power 3 or less.";
const TAP_ABILITY: &str = "{2}: Tap target creature.";
const TRAINING_GROUNDS: &str =
    "Activated abilities of creatures you control cost {2} less to activate.";
const TAP_FOUR: &str = "{4}: Tap target creature.";
const GAIN_LIFE_FOUR: &str = "{4}: You gain 1 life.";

fn mana_pool(runner: &GameRunner) -> usize {
    runner
        .state()
        .players
        .iter()
        .find(|p| p.id == P0)
        .unwrap()
        .mana_pool
        .total()
}

fn add_white_mana(s: &mut GameScenario, amount: usize) {
    s.with_mana_pool(
        P0,
        (0..amount)
            .map(|_| ManaUnit::new(ManaColor::White.into(), ObjectId(0), false, Vec::new()))
            .collect(),
    );
}

fn reduce_ability_statics(
    oracle: &str,
    name: &str,
    types: &[String],
) -> Vec<engine::types::ability::StaticDefinition> {
    parse_oracle_text(oracle, name, &[], types, &[])
        .statics
        .into_iter()
        .filter(|def| matches!(def.mode, StaticMode::ReduceAbilityCost { .. }))
        .collect()
}

fn assert_targets_creature_you_control(targets: &Option<TargetFilter>) {
    let Some(TargetFilter::Typed(typed)) = targets else {
        panic!("expected typed target filter, got {targets:?}");
    };
    assert_eq!(typed.controller, Some(ControllerRef::You));
    assert!(
        !typed.type_filters.is_empty(),
        "expected creature type filter: {typed:?}"
    );
}

#[test]
fn professor_hojo_static_parses_frequency_targets_and_turn_condition() {
    let types = vec!["Creature".to_string()];
    let parsed = parse_oracle_text(HOJO, "Professor Hojo", &[], &types, &[]);
    assert!(
        parsed.parse_warnings.is_empty(),
        "warnings: {:?}",
        parsed.parse_warnings
    );
    assert!(parsed
        .abilities
        .iter()
        .all(|a| !matches!(a.effect.as_ref(), Effect::Unimplemented { .. })));
    assert!(parsed
        .triggers
        .iter()
        .any(|trigger| trigger.mode == TriggerMode::BecomesTarget));
    let def = parsed
        .statics
        .iter()
        .find(|def| matches!(def.mode, StaticMode::ReduceAbilityCost { .. }))
        .expect("cost static");
    assert_eq!(def.condition, Some(StaticCondition::DuringYourTurn));
    let StaticMode::ReduceAbilityCost {
        mode,
        keyword,
        amount,
        activator,
        targets,
        frequency,
        ..
    } = &def.mode
    else {
        unreachable!()
    };
    assert_eq!(*mode, CostModifyMode::Reduce);
    assert_eq!(keyword, "activated");
    assert_eq!(*amount, 2);
    assert_eq!(*activator, Some(PlayerFilter::Controller));
    assert_eq!(*frequency, Some(CastFrequency::OncePerTurn));
    assert_targets_creature_you_control(targets);
}

#[test]
fn target_restricted_cost_static_parses_kopala_tezzeret_and_equip() {
    let creature = vec!["Creature".to_string()];
    let kopala = reduce_ability_statics(KOPALA, "Kopala, Warden of Waves", &creature);
    let kopala_def = kopala
        .iter()
        .find(|def| {
            matches!(
                def.mode,
                StaticMode::ReduceAbilityCost {
                    mode: CostModifyMode::Raise,
                    ..
                }
            )
        })
        .expect("Kopala activate tax");
    let StaticMode::ReduceAbilityCost {
        amount,
        activator,
        targets,
        ..
    } = &kopala_def.mode
    else {
        unreachable!()
    };
    assert_eq!(*amount, 2);
    assert_eq!(*activator, Some(PlayerFilter::Opponent));
    assert!(targets.is_some(), "Kopala target gate must parse");

    let artifact = vec!["Artifact".to_string()];
    let tezzeret = reduce_ability_statics(TEZZERET, "Tezzeret, Betrayer of Flesh", &artifact);
    let StaticMode::ReduceAbilityCost {
        frequency, targets, ..
    } = &tezzeret[0].mode
    else {
        unreachable!()
    };
    assert_eq!(*frequency, Some(CastFrequency::OncePerTurn));
    assert!(targets.is_none());

    let fervent = reduce_ability_statics(FERVENT, "Fervent Champion", &creature);
    let StaticMode::ReduceAbilityCost {
        keyword,
        amount,
        targets,
        ..
    } = &fervent[0].mode
    else {
        unreachable!()
    };
    assert_eq!(keyword, "equip");
    assert_eq!(*amount, 3);
    assert!(targets.is_some(), "equip target gate must parse");
}

#[test]
fn raft_security_officer_discount_depends_on_committed_target_power() {
    fn run(power: i32) -> usize {
        let mut s = GameScenario::new();
        s.at_phase(Phase::PreCombatMain);
        let raft = s
            .add_creature_from_oracle(P0, "Raft Security Officer", 1, 3, RAFT)
            .id();
        let victim = s.add_vanilla(P1, power, 5);
        add_white_mana(&mut s, 3);
        let mut runner = s.build();
        runner
            .state_mut()
            .objects
            .get_mut(&raft)
            .unwrap()
            .has_summoning_sickness = false;
        runner.activate(raft, 0).target_object(victim).resolve();
        mana_pool(&runner)
    }
    assert_eq!(
        run(1),
        2,
        "positive guard: qualifying target gets {{1}} discount"
    );
    assert_eq!(run(5), 1, "nonqualifying target must pay full {{2}}");
}

#[test]
fn hojo_discount_counts_first_qualifying_activation_only() {
    let mut s = GameScenario::new();
    s.at_phase(Phase::PreCombatMain);
    s.add_creature_from_oracle(P0, "Professor Hojo", 2, 2, HOJO);
    let wand = s
        .add_artifact_from_oracle(P0, "Training Wand", TAP_ABILITY)
        .id();
    let own_a = s.add_creature(P0, "Own A", 1, 1).id();
    let own_b = s.add_creature(P0, "Own B", 1, 1).id();
    s.add_card_to_library_top(P0, "Draw A");
    s.add_card_to_library_top(P0, "Draw B");
    add_white_mana(&mut s, 4);
    let mut runner = s.build();
    runner.activate(wand, 0).target_object(own_a).resolve();
    assert_eq!(
        mana_pool(&runner),
        4,
        "positive guard: first qualifying activation is free"
    );
    runner.activate(wand, 0).target_object(own_b).resolve();
    assert_eq!(
        mana_pool(&runner),
        2,
        "second qualifying activation pays {{2}}"
    );
}

#[test]
fn hojo_discount_rejects_wrong_controller_and_wrong_turn() {
    let mut s = GameScenario::new();
    s.at_phase(Phase::PreCombatMain);
    s.add_creature_from_oracle(P0, "Professor Hojo", 2, 2, HOJO);
    let wand = s
        .add_artifact_from_oracle(P0, "Training Wand", TAP_ABILITY)
        .id();
    let enemy = s.add_creature(P1, "Enemy", 1, 1).id();
    add_white_mana(&mut s, 4);
    let mut runner = s.build();
    runner.activate(wand, 0).target_object(enemy).resolve();
    assert_eq!(
        mana_pool(&runner),
        2,
        "opponent-controlled target pays full {{2}}"
    );

    let mut s = GameScenario::new();
    s.at_phase(Phase::PreCombatMain);
    s.add_creature_from_oracle(P0, "Professor Hojo", 2, 2, HOJO);
    let wand = s
        .add_artifact_from_oracle(P0, "Training Wand", TAP_ABILITY)
        .id();
    let own = s.add_creature(P0, "Own", 1, 1).id();
    s.add_card_to_library_top(P0, "Draw A");
    add_white_mana(&mut s, 4);
    let mut runner = s.build();
    runner.state_mut().active_player = P1;
    runner.activate(wand, 0).target_object(own).resolve();
    assert_eq!(mana_pool(&runner), 2, "not your turn pays full {{2}}");
}

#[test]
fn fervent_champion_equip_discount_depends_on_target() {
    let mut s = GameScenario::new();
    s.at_phase(Phase::PreCombatMain);
    let fervent = s
        .add_creature_from_oracle(P0, "Fervent Champion", 1, 1, FERVENT)
        .id();
    let other = s.add_creature(P0, "Other", 1, 1).id();
    let equipment = s
        .add_artifact_from_oracle(P0, "Practice Sword", "Equip {3}")
        .id();
    add_white_mana(&mut s, 6);
    let mut runner = s.build();
    runner
        .activate(equipment, 0)
        .target_object(fervent)
        .resolve();
    assert_eq!(
        mana_pool(&runner),
        6,
        "positive guard: Fervent equip is free"
    );
    runner.activate(equipment, 0).target_object(other).resolve();
    assert_eq!(mana_pool(&runner), 3, "other creature pays equip {{3}}");
}

/// CR 601.2f + CR 602.2b: the post-target (`Committed`) cost pass exists only to
/// fold in riders the announcement pass had to defer. A target-INDEPENDENT
/// static (Training Grounds) is already folded in at announcement, so it must
/// NOT be applied a second time when targets settle.
///
/// Regression guard for a measured defect: with the `Committed` pass applying
/// every static, a TARGETED `{4}` ability under Training Grounds paid `{0}`
/// instead of `{2}` while the untargeted path stayed correct — a silent
/// misprice with no marker and no crash. The untargeted leg is the control that
/// proves the guard is discriminating rather than blanket.
#[test]
fn target_independent_static_is_not_double_applied_after_targets_settle() {
    fn paid(ability_oracle: &str, targeted: bool) -> usize {
        let mut s = GameScenario::new();
        s.at_phase(Phase::PreCombatMain);
        s.add_artifact_from_oracle(P0, "Training Grounds", TRAINING_GROUNDS);
        let src = s
            .add_creature_from_oracle(P0, "Tapper", 2, 2, ability_oracle)
            .id();
        let victim = s.add_vanilla(P1, 1, 1);
        add_white_mana(&mut s, 6);
        let mut runner = s.build();
        runner
            .state_mut()
            .objects
            .get_mut(&src)
            .unwrap()
            .has_summoning_sickness = false;
        let before = mana_pool(&runner);
        if targeted {
            runner.activate(src, 0).target_object(victim).resolve();
        } else {
            runner.activate(src, 0).resolve();
        }
        before - mana_pool(&runner)
    }

    assert_eq!(
        paid(TAP_FOUR, true),
        2,
        "targeted {{4}} under Training Grounds pays {{2}} - a second application would make it free"
    );
    assert_eq!(
        paid(GAIN_LIFE_FOUR, false),
        2,
        "control: the untargeted path was already correct and must stay {{2}}"
    );
}
