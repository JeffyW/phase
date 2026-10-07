//! Oxford-comma subtype lists ("a Kraken, Leviathan, Merfolk, Octopus, or
//! Serpent") in trigger conditions and relative clauses, driven through the
//! production pipeline.
//!
//! Summon: Leviathan is the motivating card:
//! - chapter I's "each creature that isn't a Kraken, Leviathan, Merfolk,
//!   Octopus, or Serpent" kept only the first leg (`Non(Kraken)`), so it also
//!   bounced Leviathans, Merfolk, Octopuses and Serpents;
//! - chapters II and III ("Until end of turn, whenever a Kraken, ..., or Serpent
//!   attacks, draw a card") split the delayed trigger's condition from its effect
//!   at the list's FIRST comma, leaving an unknown trigger that never fired.
//!
//! The same condition/effect split cut printed trigger conditions before their
//! list's closing "or" leg (Fumulus, the Infestation; Spawning Kraken), and the
//! same relative-clause parser dropped Swarmyard Massacre's lists.
//!
//! Oracle text is verbatim from Scryfall.

use engine::game::combat::AttackTarget;
use engine::game::scenario::{GameRunner, GameScenario, P0, P1};
use engine::game::triggers::drain_order_triggers_with_identity;
use engine::parser::oracle_effect::parse_effect_chain;
use engine::types::ability::{
    AbilityDefinition, AbilityKind, DelayedTriggerCondition, Effect, TargetFilter,
};
use engine::types::actions::GameAction;
use engine::types::counter::CounterType;
use engine::types::game_state::WaitingFor;
use engine::types::identifiers::ObjectId;
use engine::types::phase::Phase;
use engine::types::player::PlayerId;
use engine::types::triggers::TriggerMode;
use engine::types::zones::Zone;

const SUMMON_LEVIATHAN: &str = "(As this Saga enters and after your draw step, add a lore counter. Sacrifice after III.)\n\
I \u{2014} Return each creature that isn't a Kraken, Leviathan, Merfolk, Octopus, or Serpent to its owner's hand.\n\
II, III \u{2014} Until end of turn, whenever a Kraken, Leviathan, Merfolk, Octopus, or Serpent attacks, draw a card.\n\
Ward {2}";

const FUMULUS: &str = "Flying, deathtouch\n\
Whenever a player sacrifices a nontoken creature, create a 1/1 black Insect creature token with flying.\n\
Whenever an Insect, Leech, Slug, or Worm you control attacks, defending player loses 1 life and you gain 1 life.";

const SPAWNING_KRAKEN: &str = "Whenever a Kraken, Leviathan, Octopus, or Serpent you control deals combat damage to a player, create a 9/9 blue Kraken creature token.";

const SWARMYARD_MASSACRE: &str = "Create two 1/1 green Squirrel creature tokens. Then each creature that isn't an Insect, Rat, Spider, or Squirrel gets -1/-1 until end of turn for each creature you control that's an Insect, Rat, Spider, or Squirrel.";

fn add_summon_leviathan(scenario: &mut GameScenario, lore: u32) -> ObjectId {
    let saga = scenario
        .add_creature(P0, "Summon: Leviathan", 6, 6)
        .as_enchantment()
        .with_subtypes(vec!["Saga", "Leviathan"])
        .from_oracle_text_with_keywords(&["Ward"], SUMMON_LEVIATHAN)
        .id();
    if lore > 0 {
        scenario.with_counter(saga, CounterType::Lore, lore);
    }
    saga
}

fn add_typed(scenario: &mut GameScenario, player: PlayerId, name: &str, subtype: &str) -> ObjectId {
    scenario
        .add_creature(player, name, 2, 2)
        .with_subtypes(vec![subtype])
        .id()
}

fn lore(runner: &GameRunner, saga: ObjectId) -> u32 {
    runner.state().objects[&saga]
        .counters
        .get(&CounterType::Lore)
        .copied()
        .unwrap_or(0)
}

fn hand_size(runner: &GameRunner, player: PlayerId) -> usize {
    runner.state().players[player.0 as usize].hand.len()
}

fn zone(runner: &GameRunner, id: ObjectId) -> Zone {
    runner.state().objects[&id].zone
}

/// Put every pending trigger on the stack and resolve the stack, answering only
/// trigger ordering and priority. Stops at the first other decision.
fn resolve_stack(runner: &mut GameRunner) {
    for _ in 0..64 {
        match runner.state().waiting_for.clone() {
            WaitingFor::OrderTriggers { .. } => {
                drain_order_triggers_with_identity(runner.state_mut());
            }
            WaitingFor::Priority { .. } if !runner.state().stack.is_empty() => {
                runner.act(GameAction::PassPriority).expect("pass priority");
            }
            _ => return,
        }
    }
    panic!("stack did not empty: {:?}", runner.state().waiting_for);
}

/// Start P0's turn 2 at its upkeep and advance into its precombat main, where
/// CR 714.3c adds a lore counter and the next chapter triggers; resolve it.
fn fire_next_chapter(runner: &mut GameRunner) {
    {
        let state = runner.state_mut();
        state.turn_number = 2;
        state.active_player = P0;
        state.phase = Phase::Upkeep;
        state.priority_player = P0;
        state.waiting_for = WaitingFor::Priority { player: P0 };
    }
    runner.advance_to_phase(Phase::PreCombatMain);
    resolve_stack(runner);
    assert!(
        runner.state().stack.is_empty(),
        "the chapter must resolve, waiting_for={:?}",
        runner.state().waiting_for
    );
}

/// Declare `attackers` at P0's declare-attackers step and resolve the attack
/// triggers.
fn attack(runner: &mut GameRunner, attackers: &[ObjectId]) {
    runner.advance_to_combat();
    let attacks: Vec<_> = attackers
        .iter()
        .map(|&id| (id, AttackTarget::Player(P1)))
        .collect();
    runner
        .declare_attackers(&attacks)
        .expect("declare attackers");
    resolve_stack(runner);
}

/// Pass through the rest of the turn, answering every decision with "nothing",
/// until `stop` holds.
fn drive_until(runner: &mut GameRunner, stop: impl Fn(&GameRunner) -> bool) {
    for _ in 0..400 {
        if stop(runner) {
            return;
        }
        match runner.state().waiting_for.clone() {
            WaitingFor::OrderTriggers { .. } => {
                drain_order_triggers_with_identity(runner.state_mut());
            }
            WaitingFor::DeclareAttackers { .. } => {
                runner.declare_attackers(&[]).expect("declare no attackers");
            }
            WaitingFor::DeclareBlockers { .. } => {
                runner
                    .act(GameAction::DeclareBlockers {
                        assignments: vec![],
                    })
                    .expect("declare no blockers");
            }
            WaitingFor::Priority { .. } => {
                runner.act(GameAction::PassPriority).expect("pass priority");
            }
            other => panic!("unexpected decision while driving the game: {other:?}"),
        }
    }
    panic!(
        "never reached the stop condition: phase={:?} active={:?} waiting_for={:?}",
        runner.state().phase,
        runner.state().active_player,
        runner.state().waiting_for
    );
}

/// CR 205.3m + CR 608.2c: chapter I returns every creature that is none of the
/// five listed creature types, whoever controls it, and leaves each listed type
/// (including the Saga, itself a Leviathan) on the battlefield.
#[test]
fn chapter_one_returns_only_creatures_outside_the_listed_types() {
    let mut scenario = GameScenario::new();
    scenario.at_phase(Phase::PreCombatMain);
    let saga = add_summon_leviathan(&mut scenario, 0);
    let kraken = add_typed(&mut scenario, P1, "Kraken", "Kraken");
    let leviathan = add_typed(&mut scenario, P1, "Leviathan", "Leviathan");
    let merfolk = add_typed(&mut scenario, P0, "Merfolk", "Merfolk");
    let octopus = add_typed(&mut scenario, P1, "Octopus", "Octopus");
    let serpent = add_typed(&mut scenario, P0, "Serpent", "Serpent");
    let bear = add_typed(&mut scenario, P0, "Bear", "Bear");
    let human = add_typed(&mut scenario, P1, "Human", "Human");
    scenario.with_library_top(P0, &["Card A", "Card B"]);
    let mut runner = scenario.build();

    fire_next_chapter(&mut runner);
    assert_eq!(lore(&runner, saga), 1, "chapter I fired");

    // Positive reach guard: chapter I resolved and bounced the unlisted types.
    assert_eq!(zone(&runner, bear), Zone::Hand, "Bear is not a listed type");
    assert_eq!(
        zone(&runner, human),
        Zone::Hand,
        "Human is not a listed type"
    );
    for (id, name) in [
        (saga, "Summon: Leviathan"),
        (kraken, "Kraken"),
        (leviathan, "Leviathan"),
        (merfolk, "Merfolk"),
        (octopus, "Octopus"),
        (serpent, "Serpent"),
    ] {
        assert_eq!(
            zone(&runner, id),
            Zone::Battlefield,
            "{name} is a listed type and must stay"
        );
    }
}

/// CR 603.7b + CR 508.3a + CR 603.2c: chapter II creates a delayed trigger with a
/// stated duration that fires once per listed creature declared as an attacker
/// that turn, and not for an unlisted attacker.
#[test]
fn chapter_two_draws_once_per_listed_attacker() {
    let mut scenario = GameScenario::new();
    scenario.at_phase(Phase::PreCombatMain);
    let saga = add_summon_leviathan(&mut scenario, 1);
    let merfolk = add_typed(&mut scenario, P0, "Merfolk", "Merfolk");
    let serpent = add_typed(&mut scenario, P0, "Serpent", "Serpent");
    let bear = add_typed(&mut scenario, P0, "Bear", "Bear");
    scenario.with_library_top(P0, &["Card A", "Card B", "Card C", "Card D", "Card E"]);
    let mut runner = scenario.build();

    fire_next_chapter(&mut runner);
    assert_eq!(lore(&runner, saga), 2, "chapter II fired");
    assert_eq!(
        runner.state().delayed_triggers.len(),
        1,
        "chapter II must create its delayed trigger"
    );

    let hand_before = hand_size(&runner, P0);
    attack(&mut runner, &[merfolk, serpent, bear]);
    assert_eq!(
        hand_size(&runner, P0),
        hand_before + 2,
        "one draw for the Merfolk and one for the Serpent; none for the Bear"
    );
}

/// CR 603.7c + CR 714.4: chapter III's delayed trigger is independent of its
/// source. The Saga is sacrificed as soon as chapter III leaves the stack, and a
/// Kraken attacking later that turn still draws its controller a card.
#[test]
fn chapter_three_trigger_outlives_the_sacrificed_saga() {
    let mut scenario = GameScenario::new();
    scenario.at_phase(Phase::PreCombatMain);
    let saga = add_summon_leviathan(&mut scenario, 2);
    let kraken = add_typed(&mut scenario, P0, "Kraken", "Kraken");
    scenario.with_library_top(P0, &["Card A", "Card B", "Card C"]);
    let mut runner = scenario.build();

    fire_next_chapter(&mut runner);
    assert_eq!(
        zone(&runner, saga),
        Zone::Graveyard,
        "CR 714.4: the Saga is sacrificed after chapter III resolves"
    );
    assert_eq!(runner.state().delayed_triggers.len(), 1);

    let hand_before = hand_size(&runner, P0);
    attack(&mut runner, &[kraken]);
    assert_eq!(
        hand_size(&runner, P0),
        hand_before + 1,
        "the delayed trigger still fires after its source left the battlefield"
    );
}

/// CR 603.7b + CR 514.2: "until end of turn" ends the delayed trigger at
/// cleanup. The trigger names no controller, so had it survived, the
/// opponent's Octopus attacking on the next turn would draw P0 a card.
#[test]
fn chapter_two_trigger_ends_at_cleanup() {
    let mut scenario = GameScenario::new();
    scenario.at_phase(Phase::PreCombatMain);
    let saga = add_summon_leviathan(&mut scenario, 1);
    let merfolk = add_typed(&mut scenario, P0, "Merfolk", "Merfolk");
    let octopus = add_typed(&mut scenario, P1, "Octopus", "Octopus");
    scenario.with_library_top(P0, &["Card A", "Card B", "Card C"]);
    scenario.with_library_top(P1, &["Card X", "Card Y"]);
    let mut runner = scenario.build();

    fire_next_chapter(&mut runner);
    assert_eq!(lore(&runner, saga), 2, "chapter II fired");
    // Positive control: the trigger is live this turn.
    let hand_before = hand_size(&runner, P0);
    attack(&mut runner, &[merfolk]);
    assert_eq!(hand_size(&runner, P0), hand_before + 1, "live this turn");

    drive_until(&mut runner, |r| {
        r.state().active_player == P1
            && matches!(
                r.state().waiting_for,
                WaitingFor::DeclareAttackers { player, .. } if player == P1
            )
    });
    assert!(
        runner.state().delayed_triggers.is_empty(),
        "CR 514.2: the delayed trigger must be gone after cleanup, got {:?}",
        runner.state().delayed_triggers
    );

    let p0_hand = hand_size(&runner, P0);
    runner
        .declare_attackers(&[(octopus, AttackTarget::Player(P0))])
        .expect("P1 attacks with its Octopus");
    resolve_stack(&mut runner);
    assert_eq!(
        hand_size(&runner, P0),
        p0_hand,
        "the expired trigger must not fire on the opponent's turn"
    );
}

/// CR 508.3a + CR 603.2c: Fumulus's trigger covers every leg of its list,
/// including the closing "or Worm", once per attacking creature of those types.
#[test]
fn fumulus_triggers_for_the_closing_leg_of_its_list() {
    let mut scenario = GameScenario::new();
    scenario.at_phase(Phase::PreCombatMain);
    scenario
        .add_creature(P0, "Fumulus, the Infestation", 2, 2)
        .as_legendary()
        .with_subtypes(vec!["Vampire", "Insect"])
        .with_summoning_sickness()
        .from_oracle_text_with_keywords(&["Flying", "Deathtouch"], FUMULUS);
    let worm = add_typed(&mut scenario, P0, "Worm", "Worm");
    let leech = add_typed(&mut scenario, P0, "Leech", "Leech");
    let bear = add_typed(&mut scenario, P0, "Bear", "Bear");
    let mut runner = scenario.build();

    let (p0_life, p1_life) = (runner.life(P0), runner.life(P1));
    attack(&mut runner, &[worm, leech, bear]);
    assert_eq!(runner.life(P1), p1_life - 2, "Worm and Leech each drain 1");
    assert_eq!(runner.life(P0), p0_life + 2, "Worm and Leech each gain 1");
}

/// CR 510.2 + CR 603.2c: Spawning Kraken's trigger covers its closing "or
/// Serpent" leg. A Serpent dealing combat damage to a player creates a 9/9 Kraken;
/// an unlisted creature dealing combat damage beside it does not.
#[test]
fn spawning_kraken_triggers_for_the_closing_leg_of_its_list() {
    let mut scenario = GameScenario::new();
    scenario.at_phase(Phase::PreCombatMain);
    scenario
        .add_creature(P0, "Spawning Kraken", 6, 6)
        .with_subtypes(vec!["Kraken"])
        .with_summoning_sickness()
        .from_oracle_text(SPAWNING_KRAKEN);
    let serpent = add_typed(&mut scenario, P0, "Serpent", "Serpent");
    let bear = add_typed(&mut scenario, P0, "Bear", "Bear");
    let mut runner = scenario.build();

    let krakens = |r: &GameRunner| {
        r.state()
            .objects
            .values()
            .filter(|o| o.is_token && o.zone == Zone::Battlefield && o.name == "Kraken")
            .count()
    };
    attack(&mut runner, &[serpent, bear]);
    drive_until(&mut runner, |r| r.state().phase == Phase::PostCombatMain);
    resolve_stack(&mut runner);
    // Positive reach guard: combat damage was dealt.
    assert_eq!(
        runner.life(P1),
        20 - 4,
        "both attackers dealt combat damage"
    );
    assert_eq!(
        krakens(&runner),
        1,
        "one Kraken token, for the Serpent only"
    );
}

/// CR 205.3m + CR 608.2h: Swarmyard Massacre's lists are read whole. Every
/// creature that is none of the four types gets -N/-N, where N counts the
/// listed creatures its caster controls (two Squirrel tokens plus a Rat = 3).
#[test]
fn swarmyard_massacre_reads_both_lists_whole() {
    let mut scenario = GameScenario::new();
    scenario.at_phase(Phase::PreCombatMain);
    let rat = add_typed(&mut scenario, P0, "Rat", "Rat");
    let spider = add_typed(&mut scenario, P1, "Spider", "Spider");
    let insect = add_typed(&mut scenario, P1, "Insect", "Insect");
    let giant = scenario
        .add_creature(P1, "Giant", 5, 5)
        .with_subtypes(vec!["Giant"])
        .id();
    let massacre = scenario
        .add_spell_to_hand_from_oracle(P0, "Swarmyard Massacre", false, SWARMYARD_MASSACRE)
        .id();
    let mut runner = scenario.build();

    runner.cast(massacre).resolve();

    let giant_obj = &runner.state().objects[&giant];
    assert_eq!(
        (giant_obj.power, giant_obj.toughness),
        (Some(2), Some(2)),
        "Giant gets -3/-3 (Rat + two Squirrels)"
    );
    for (id, name) in [(rat, "Rat"), (spider, "Spider"), (insect, "Insect")] {
        let obj = &runner.state().objects[&id];
        assert_eq!(
            (obj.power, obj.toughness),
            (Some(2), Some(2)),
            "{name} is a listed type and is unaffected"
        );
    }
}

/// The delayed trigger's effect: the `CreateDelayedTrigger` inner ability of a
/// one-line spell.
fn delayed_body(text: &str) -> AbilityDefinition {
    let def = parse_effect_chain(text, AbilityKind::Spell);
    let mut cur = &def;
    loop {
        if let Effect::CreateDelayedTrigger { effect, .. } = &*cur.effect {
            return (**effect).clone();
        }
        cur = cur
            .sub_ability
            .as_deref()
            .unwrap_or_else(|| panic!("no CreateDelayedTrigger in {def:?}"));
    }
}

/// CR 509.3c + CR 608.2k: in a bare "becomes blocked" condition, "it" names the
/// blocked attacker. The (blocker, attacker) event's `TriggeringSource` is the
/// blocker, so the body binds `ParentTarget`, which resolves the attacker.
#[test]
fn bare_becomes_blocked_it_binds_the_attacker_not_the_blocker() {
    let body = delayed_body(
        "Until end of turn, whenever a creature becomes blocked, it gets +1/+1 until end of turn.",
    );
    let Effect::Pump { target, .. } = &*body.effect else {
        panic!("expected Pump, got {:?}", body.effect);
    };
    assert_eq!(*target, TargetFilter::ParentTarget);
}

/// Runtime: a single blocker. The attacker gets +1/+1 and the blocker doesn't.
#[test]
fn bare_becomes_blocked_pumps_the_attacker_on_a_printed_trigger() {
    let mut scenario = GameScenario::new();
    scenario.at_phase(Phase::PreCombatMain);
    scenario
        .add_creature(P0, "Somberwald Alpha", 3, 2)
        .with_subtypes(vec!["Wolf"])
        .with_summoning_sickness()
        .from_oracle_text(
            "Whenever a creature you control becomes blocked, it gets +1/+1 until end of turn.",
        );
    let attacker = scenario.add_creature(P0, "Attacker", 2, 2).id();
    let blocker = scenario.add_creature(P1, "Blocker", 2, 2).id();
    let mut runner = scenario.build();

    attack(&mut runner, &[attacker]);
    drive_until(&mut runner, |r| {
        matches!(r.state().waiting_for, WaitingFor::DeclareBlockers { .. })
    });
    runner
        .act(GameAction::DeclareBlockers {
            assignments: vec![(blocker, attacker)],
        })
        .expect("declare the block");
    resolve_stack(&mut runner);

    runner.state_mut().layers_dirty.mark_full();
    engine::game::layers::evaluate_layers(runner.state_mut());
    let pt = |id: ObjectId| {
        let o = &runner.state().objects[&id];
        (o.power, o.toughness)
    };
    assert_eq!(
        pt(attacker),
        (Some(3), Some(3)),
        "the blocked attacker is pumped"
    );
    assert_eq!(pt(blocker), (Some(2), Some(2)), "the blocker is not");
}

/// CR 603.1: a type list followed by a bare action-verb effect. "investigate"
/// is not another list item, so the boundary is the comma before it.
#[test]
fn type_list_then_action_verb_effect_splits_before_the_action() {
    let def = parse_effect_chain(
        "Until end of turn, whenever a Kraken, or Serpent attacks, investigate.",
        AbilityKind::Spell,
    );
    let Effect::CreateDelayedTrigger {
        condition: DelayedTriggerCondition::WheneverEvent { trigger, .. },
        effect,
        ..
    } = &*def.effect
    else {
        panic!(
            "expected a WheneverEvent delayed trigger, got {:?}",
            def.effect
        );
    };
    assert_eq!(trigger.mode, TriggerMode::Attacks);
    assert!(matches!(&*effect.effect, Effect::Investigate));
}

/// Control: Mistway Spy's delayed trigger still splits before "investigate".
#[test]
fn mistway_spy_delayed_trigger_still_investigates() {
    let body = delayed_body(
        "Until end of turn, whenever a creature you control deals combat damage to a player, investigate.",
    );
    assert!(matches!(&*body.effect, Effect::Investigate), "{body:?}");
}

/// CR 603.4: a delayed body whose leading intervening "if" the parser drops
/// fails closed. Reach guard: the delayed trigger itself still parses.
#[test]
fn delayed_body_with_a_dropped_intervening_if_is_unimplemented() {
    let body = delayed_body(
        "Whenever you cast a spell this turn, if this card is suspended, remove a time counter from it.",
    );
    assert!(
        matches!(&*body.effect, Effect::Unimplemented { name, .. } if name == "delayed_intervening_if_dropped"),
        "{body:?}"
    );
}

/// Control: a recognized intervening "if" is kept on the delayed body's root.
#[test]
fn delayed_body_with_a_recognized_intervening_if_keeps_it() {
    let body = delayed_body(
        "Until end of turn, whenever a creature dies, if you control a Human, draw a card.",
    );
    assert!(body.condition.is_some(), "{body:?}");
    assert!(matches!(&*body.effect, Effect::Draw { .. }), "{body:?}");
}

fn has_unimplemented(def: &AbilityDefinition, gap: &str) -> bool {
    serde_json::to_string(def)
        .expect("serialize")
        .contains(&format!("\"name\":\"{gap}\""))
}

/// CR 509.3c + CR 120.1: under a bare "becomes blocked" condition, "it deals N
/// damage" names the blocked attacker as the damage source. No `DamageSource`
/// value can name it, so the clause fails closed instead of being dealt by the
/// Equipment. Reach guard: the trigger is a `BecomesBlocked` trigger whose
/// "it" pin is live (a target-position "it" in the same shape binds
/// `ParentTarget`, asserted by `bare_becomes_blocked_it_binds_the_attacker_not_the_blocker`).
#[test]
fn bare_becomes_blocked_it_as_damage_source_fails_closed() {
    let parsed = engine::parser::oracle::parse_oracle_text(
        "Equipped creature gets +1/+1.\nWhenever equipped creature becomes blocked, it deals 1 damage to defending player.\nEquip {1}",
        "Tormentor's Helm",
        &["Equip".to_string()],
        &["Artifact".to_string()],
        &["Equipment".to_string()],
    );
    let trigger = parsed
        .triggers
        .iter()
        .find(|t| t.mode == TriggerMode::BecomesBlocked)
        .expect("BecomesBlocked trigger");
    let execute = trigger.execute.as_deref().expect("execute");
    assert!(
        has_unimplemented(execute, "blocked_attacker_damage_source"),
        "{execute:?}"
    );
}

/// CR 701.23a: a search zone list with a leg outside the zone vocabulary is
/// not represented by searching only the recognized zones. Invasion of
/// Arcavios stays unsupported, with no unconditional shuffle.
#[test]
fn search_with_an_unrecognized_zone_leg_fails_closed() {
    let parsed = engine::parser::oracle::parse_oracle_text(
        "(As a Siege enters, choose an opponent to protect it. You and others can attack it. When it's defeated, exile it, then cast it transformed.)\nWhen this Siege enters, search your library, graveyard, and/or outside the game for an instant or sorcery card you own, reveal it, and put it into your hand. If you search your library this way, shuffle.",
        "Invasion of Arcavios",
        &[],
        &["Battle".to_string()],
        &["Siege".to_string()],
    );
    let trigger = parsed
        .triggers
        .iter()
        .find(|t| t.destination == Some(Zone::Battlefield))
        .expect("ETB trigger");
    let json = serde_json::to_string(trigger).expect("serialize");
    assert!(json.contains("\"search_zone_list\""), "{json}");
    assert!(
        !json.contains("\"SearchLibrary\""),
        "no partial search of only the recognized zones: {json}"
    );
}

/// Control: a fully recognized multi-zone search keeps all its zones.
#[test]
fn search_with_a_recognized_zone_list_keeps_every_zone() {
    let def = parse_effect_chain(
        "Search your graveyard, hand, and/or library for a card named God-Pharaoh's Gift and put it onto the battlefield. If you search your library this way, shuffle.",
        AbilityKind::Spell,
    );
    let json = serde_json::to_string(&def).expect("serialize");
    assert!(
        json.contains("\"source_zones\":[\"Graveyard\",\"Hand\",\"Library\"]"),
        "{json}"
    );
}

fn printed_trigger_json(oracle: &str, name: &str, types: &[&str], subtypes: &[&str]) -> String {
    let s = |v: &[&str]| v.iter().map(|x| x.to_string()).collect::<Vec<_>>();
    let parsed =
        engine::parser::oracle::parse_oracle_text(oracle, name, &[], &s(types), &s(subtypes));
    serde_json::to_string(&parsed.triggers).expect("serialize")
}

/// D7 rows: "it" as a damage source under a bare "becomes blocked" fails
/// closed — in a conditional sub-clause (Ib Halfheart) and in "have it deal"
/// (Laccolith Rig). Reach guard: each parses a `BecomesBlocked` trigger.
#[test]
fn blocked_attacker_damage_source_fails_closed_in_subclause_and_have_it_deal() {
    for (name, oracle, types, subtypes) in [
        (
            "Ib Halfheart, Goblin Tactician",
            "Whenever another Goblin you control becomes blocked, sacrifice it. If you do, it deals 4 damage to each creature blocking it.\nSacrifice two Mountains: Create two 1/1 red Goblin creature tokens.",
            &["Creature"][..],
            &["Goblin", "Advisor"][..],
        ),
        (
            "Laccolith Rig",
            "Enchant creature\nWhenever enchanted creature becomes blocked, you may have it deal damage equal to its power to target creature. If you do, the first creature assigns no combat damage this turn.",
            &["Enchantment"][..],
            &["Aura"][..],
        ),
    ] {
        let json = printed_trigger_json(oracle, name, types, subtypes);
        assert!(json.contains("\"BecomesBlocked\""), "{name}: {json}");
        assert!(
            json.contains("\"blocked_attacker_damage_source\""),
            "{name}: {json}"
        );
    }
}

/// D7, delayed-synthetic row (labelled synthetic: no printed card has this
/// shape): the same predicate fails a delayed body closed.
#[test]
fn synthetic_delayed_blocked_attacker_damage_source_fails_closed() {
    let body = delayed_body(
        "Until end of turn, whenever a creature becomes blocked, it deals 1 damage to defending player.",
    );
    assert!(
        has_unimplemented(&body, "blocked_attacker_damage_source"),
        "{body:?}"
    );
}

/// D7 controls under the same bare "becomes blocked" condition: a self
/// source (Close Quarters' enchantment), a self subject, and an explicitly
/// targeted source all stay supported.
#[test]
fn blocked_attacker_damage_source_controls_stay_supported() {
    for (name, oracle, types) in [
        (
            "Close Quarters",
            "Whenever a creature you control becomes blocked, this enchantment deals 1 damage to any target.",
            &["Enchantment"][..],
        ),
        (
            "Self Subject Probe",
            "Whenever this creature becomes blocked, it deals 1 damage to defending player.",
            &["Creature"][..],
        ),
        (
            "Targeted Source Probe",
            "Whenever a creature you control becomes blocked, target creature you control deals 1 damage to any target.",
            &["Enchantment"][..],
        ),
    ] {
        let json = printed_trigger_json(oracle, name, types, &[]);
        assert!(json.contains("\"DealDamage\""), "{name}: {json}");
        assert!(!json.contains("\"Unimplemented\""), "{name}: {json}");
    }
}

/// D8: Turtles Forever ("search your library and/or outside the game") and a
/// non-library-leading list with an unreadable leg both fail closed with the
/// typed gap; a valid single-zone search stays library-only.
#[test]
fn search_zone_list_controls() {
    for text in [
        "Search your library and/or outside the game for exactly four legendary creature cards you own with different names, then reveal those cards. An opponent chooses two of them. Put the chosen cards into your hand and shuffle the rest into your library.",
        "Search your graveyard and/or outside the game for a creature card, reveal it, and put it into your hand.",
        "When this Siege enters, search your library, graveyard, and/or outside the game for an instant or sorcery card you own, reveal it, and put it into your hand. If you search your library this way, shuffle.",
    ] {
        let def = parse_effect_chain(text, AbilityKind::Spell);
        assert!(has_unimplemented(&def, "search_zone_list"), "{text}: {def:?}");
    }
    let single = parse_effect_chain(
        "Search your library for a creature card, reveal it, put it into your hand, then shuffle.",
        AbilityKind::Spell,
    );
    let json = serde_json::to_string(&single).expect("serialize");
    // `source_zones` is skipped when it is the library-only default.
    assert!(json.contains("\"SearchLibrary\""), "{json}");
    assert!(!json.contains("\"source_zones\""), "{json}");
    assert!(!json.contains("\"Unimplemented\""), "{json}");
}

/// Item 4 (CR 603.1): a completed condition disjunction ("you scry or
/// surveil") followed by a bare action effect ends the condition at the comma.
#[test]
fn completed_action_disjunction_then_action_effect_splits() {
    let body = delayed_body("Until end of turn, whenever you scry or surveil, investigate.");
    assert!(matches!(&*body.effect, Effect::Investigate), "{body:?}");
}

/// Control: a genuine condition-side action list keeps its commas.
#[test]
fn condition_side_action_list_keeps_its_commas() {
    let json = printed_trigger_json(
        "Whenever you waterbend, earthbend, firebend, or airbend, draw a card.",
        "Bending Probe",
        &["Enchantment"],
        &[],
    );
    assert!(json.contains("\"Draw\""), "{json}");
    assert!(!json.contains("\"Unimplemented\""), "{json}");
}

/// D5 boundary controls: a delayed body whose "if" is not leading isn't
/// touched; a printed trigger's recognized intervening "if" is kept.
#[test]
fn intervening_if_strict_fail_boundary_controls() {
    let body = delayed_body(
        "Until end of turn, whenever a creature dies, draw a card if you control a Human.",
    );
    assert!(
        !has_unimplemented(&body, "delayed_intervening_if_dropped"),
        "{body:?}"
    );
    // Positive guard on the same input: the body is the Draw itself, with no
    // gap anywhere in it.
    assert!(matches!(&*body.effect, Effect::Draw { .. }), "{body:?}");
    assert!(
        !serde_json::to_string(&body)
            .expect("serialize")
            .contains("\"Unimplemented\""),
        "{body:?}"
    );
    let parsed = engine::parser::oracle::parse_oracle_text(
        "Whenever you cast a spell, if you control an artifact, draw a card.",
        "Printed Conditional Probe",
        &[],
        &["Enchantment".to_string()],
        &[],
    );
    let trigger = &parsed.triggers[0];
    let execute = trigger.execute.as_deref().expect("execute");
    assert!(
        trigger.condition.is_some() || execute.condition.is_some(),
        "{trigger:?}"
    );
    let json = serde_json::to_string(trigger).expect("serialize");
    assert!(!json.contains("\"Unimplemented\""), "{json}");
}

/// Resolve the stack answering ordering, optional "you may" (accept), target
/// prompts (first legal) and proliferate choices (`proliferate_pick`).
fn resolve_stack_accepting(runner: &mut GameRunner, proliferate_pick: &[ObjectId]) {
    use engine::types::ability::TargetRef;
    for _ in 0..64 {
        match runner.state().waiting_for.clone() {
            WaitingFor::OrderTriggers { .. } => {
                drain_order_triggers_with_identity(runner.state_mut());
            }
            WaitingFor::OptionalEffectChoice { .. } => {
                runner
                    .act(GameAction::DecideOptionalEffect { accept: true })
                    .expect("accept the optional effect");
            }
            WaitingFor::ProliferateChoice { .. } => {
                runner
                    .act(GameAction::SelectTargets {
                        targets: proliferate_pick
                            .iter()
                            .map(|id| TargetRef::Object(*id))
                            .collect(),
                    })
                    .expect("proliferate choice");
            }
            WaitingFor::TargetSelection { .. } | WaitingFor::TriggerTargetSelection { .. } => {
                runner
                    .choose_first_legal_target()
                    .expect("choose the first legal target");
            }
            WaitingFor::Priority { .. } if !runner.state().stack.is_empty() => {
                runner.act(GameAction::PassPriority).expect("pass priority");
            }
            _ => return,
        }
    }
    panic!("stack did not empty: {:?}", runner.state().waiting_for);
}

/// Declare `attacker` at P1 and have P1 block it with `blocker`.
fn attack_and_block(runner: &mut GameRunner, attacker: ObjectId, blocker: ObjectId) {
    attack(runner, &[attacker]);
    drive_until(runner, |r| {
        matches!(r.state().waiting_for, WaitingFor::DeclareBlockers { .. })
    });
    runner
        .act(GameAction::DeclareBlockers {
            assignments: vec![(blocker, attacker)],
        })
        .expect("declare the block");
}

/// CR 509.3c + CR 608.2k: Cunning Evasion returns the BLOCKED attacker, not
/// its blocker.
#[test]
fn cunning_evasion_returns_the_blocked_attacker() {
    let mut scenario = GameScenario::new();
    scenario.at_phase(Phase::PreCombatMain);
    scenario.add_enchantment_from_oracle(
        P0,
        "Cunning Evasion",
        "Whenever a creature you control becomes blocked, you may return it to its owner's hand.",
    );
    let attacker = scenario.add_creature(P0, "Attacker", 2, 2).id();
    let blocker = scenario.add_creature(P1, "Blocker", 2, 2).id();
    let mut runner = scenario.build();

    attack_and_block(&mut runner, attacker, blocker);
    resolve_stack_accepting(&mut runner, &[]);

    assert_eq!(zone(&runner, attacker), Zone::Hand, "the attacker returns");
    assert_eq!(
        zone(&runner, blocker),
        Zone::Battlefield,
        "the blocker stays"
    );
}

/// CR 701.34a + CR 603.1: "When Roalesk dies, proliferate, then proliferate
/// again" proliferates twice.
#[test]
fn roalesk_proliferates_twice() {
    let mut scenario = GameScenario::new();
    scenario.at_phase(Phase::PreCombatMain);
    let roalesk = scenario
        .add_creature(P0, "Roalesk, Apex Hybrid", 4, 5)
        .as_legendary()
        .with_subtypes(vec!["Human", "Mutant"])
        .from_oracle_text_with_keywords(
            &["Flying", "Trample"],
            "Flying, trample\nWhen Roalesk enters, put two +1/+1 counters on another target creature you control.\nWhen Roalesk dies, proliferate, then proliferate again.",
        )
        .id();
    let grower = scenario.add_creature(P0, "Grower", 1, 1).id();
    scenario.with_counter(grower, CounterType::Plus1Plus1, 1);
    let kill = scenario
        .add_spell_to_hand_from_oracle(P0, "Murder", true, "Destroy target creature.")
        .id();
    let mut runner = scenario.build();

    runner.cast(kill).target_object(roalesk).resolve();
    resolve_stack_accepting(&mut runner, &[grower]);

    // Positive reach guard: Roalesk died.
    assert_eq!(zone(&runner, roalesk), Zone::Graveyard);
    assert_eq!(
        runner.state().objects[&grower].counters[&CounterType::Plus1Plus1],
        3,
        "1 counter + two proliferates"
    );
}

/// CR 603.7b + CR 603.1: production witness for the delayed boundary. A spell
/// installs "Until end of turn, whenever a Kraken, or Serpent attacks,
/// investigate."; a Serpent attacking investigates once (a Clue). Synthetic
/// Oracle text (the list-plus-action-tail shape the boundary fix covers).
#[test]
fn delayed_list_then_action_tail_investigates_at_runtime() {
    let mut scenario = GameScenario::new();
    scenario.at_phase(Phase::PreCombatMain);
    let serpent = add_typed(&mut scenario, P0, "Serpent", "Serpent");
    let spell = scenario
        .add_spell_to_hand_from_oracle(
            P0,
            "Tidal Omen",
            false,
            "Until end of turn, whenever a Kraken, or Serpent attacks, investigate.",
        )
        .id();
    let mut runner = scenario.build();

    runner.cast(spell).resolve();
    assert_eq!(
        runner.state().delayed_triggers.len(),
        1,
        "the spell installs its delayed trigger"
    );
    let clues = |r: &GameRunner| {
        r.state()
            .objects
            .values()
            .filter(|o| o.zone == Zone::Battlefield && o.name == "Clue")
            .count()
    };
    attack(&mut runner, &[serpent]);
    assert_eq!(clues(&runner), 1, "the Serpent's attack investigates once");
}

const ROYAL_DECREE: &str = "Cumulative upkeep {W}\nWhenever a Swamp, Mountain, black permanent, or red permanent becomes tapped, this enchantment deals 1 damage to that permanent's controller.";
const ACT_OF_TREASON: &str = "Gain control of target creature until end of turn. Untap that creature. It gains haste until end of turn.";
const UNSUMMON: &str = "Return target creature to its owner's hand.";

/// A Royal Decree board: P0 controls the enchantment; P1 owns a red creature.
fn royal_decree_board() -> (GameScenario, ObjectId, ObjectId, ObjectId) {
    let mut scenario = GameScenario::new();
    scenario.at_phase(Phase::PreCombatMain);
    scenario.add_enchantment_from_oracle(P0, "Royal Decree", ROYAL_DECREE);
    let red = scenario
        .add_creature(P1, "Red Raider", 2, 2)
        .with_color(vec![engine::types::mana::ManaColor::Red])
        .id();
    let steal = scenario
        .add_spell_to_hand_from_oracle(P0, "Act of Treason", false, ACT_OF_TREASON)
        .with_mana_cost(engine::types::mana::ManaCost::zero())
        .id();
    let bounce = scenario
        .add_spell_to_hand_from_oracle(P0, "Unsummon", true, UNSUMMON)
        .with_mana_cost(engine::types::mana::ManaCost::zero())
        .id();
    (scenario, red, steal, bounce)
}

/// Steal the red creature and attack with it; its tapping triggers Royal
/// Decree, and the trigger is waiting on the stack.
fn steal_and_attack(runner: &mut GameRunner, red: ObjectId, steal: ObjectId) {
    runner.cast(steal).target_object(red).resolve();
    assert_eq!(
        runner.state().objects[&red].controller,
        P0,
        "reach guard: Act of Treason gave P0 control"
    );
    runner.advance_to_combat();
    runner
        .declare_attackers(&[(red, AttackTarget::Player(P1))])
        .expect("attack with the stolen creature");
    for _ in 0..8 {
        match runner.state().waiting_for.clone() {
            WaitingFor::OrderTriggers { .. } => {
                drain_order_triggers_with_identity(runner.state_mut());
            }
            _ => break,
        }
    }
    assert_eq!(
        runner.state().stack.len(),
        1,
        "reach guard: exactly one Royal Decree trigger for the tapped attacker"
    );
}

/// CR 608.2h + CR 109.4: "that permanent's controller" reads the controller as
/// the permanent last existed on the battlefield. The stolen red creature is
/// bounced to its owner's hand before the trigger resolves; P0, who controlled
/// it when it became tapped, takes the damage, not its owner.
#[test]
fn royal_decree_damages_the_last_controller_of_a_departed_permanent() {
    let (scenario, red, steal, bounce) = royal_decree_board();
    let mut runner = scenario.build();
    steal_and_attack(&mut runner, red, steal);

    runner.cast(bounce).target_object(red).commit();
    runner.resolve_top();
    assert_eq!(zone(&runner, red), Zone::Hand, "reach guard: bounced");
    runner.resolve_top();

    assert_eq!(runner.life(P0), 19, "P0 controlled it when it tapped");
    assert_eq!(runner.life(P1), 20, "its owner takes nothing");
}

/// Control: the stolen creature stays — its current controller (P0) is hit.
#[test]
fn royal_decree_damages_the_controller_of_a_stolen_permanent_that_stays() {
    let (scenario, red, steal, _) = royal_decree_board();
    let mut runner = scenario.build();
    steal_and_attack(&mut runner, red, steal);
    runner.resolve_top();
    assert_eq!(runner.life(P0), 19);
    assert_eq!(runner.life(P1), 20);
}

/// Control: P0's own red creature taps and is bounced before resolution —
/// P0 is hit, unchanged by the departure.
#[test]
fn royal_decree_damages_the_controller_of_its_own_departed_permanent() {
    let mut scenario = GameScenario::new();
    scenario.at_phase(Phase::PreCombatMain);
    scenario.add_enchantment_from_oracle(P0, "Royal Decree", ROYAL_DECREE);
    let red = scenario
        .add_creature(P0, "Red Raider", 2, 2)
        .with_color(vec![engine::types::mana::ManaColor::Red])
        .id();
    let bounce = scenario
        .add_spell_to_hand_from_oracle(P0, "Unsummon", true, UNSUMMON)
        .with_mana_cost(engine::types::mana::ManaCost::zero())
        .id();
    let mut runner = scenario.build();
    runner.advance_to_combat();
    runner
        .declare_attackers(&[(red, AttackTarget::Player(P1))])
        .expect("attack");
    assert_eq!(runner.state().stack.len(), 1, "reach guard: one trigger");
    runner.cast(bounce).target_object(red).commit();
    runner.resolve_top();
    runner.resolve_top();
    assert_eq!(runner.life(P0), 19);
    assert_eq!(runner.life(P1), 20);
}

/// Official ruling: Royal Decree triggers at most once for each permanent that
/// becomes tapped, even if it meets several criteria. A Swamp Mountain taps →
/// one trigger, 1 damage to its controller (P1).
#[test]
fn royal_decree_triggers_once_for_a_permanent_meeting_several_criteria() {
    let mut scenario = GameScenario::new();
    scenario.at_phase(Phase::PreCombatMain);
    scenario.add_enchantment_from_oracle(P0, "Royal Decree", ROYAL_DECREE);
    let dual = scenario
        .add_land_from_oracle(P1, "Swamp Mountain", "{T}: Add {B}.")
        .with_subtypes(vec!["Swamp", "Mountain"])
        .id();
    let mut runner = scenario.build();
    {
        let state = runner.state_mut();
        state.active_player = P1;
        state.priority_player = P1;
        state.waiting_for = WaitingFor::Priority { player: P1 };
    }
    runner.activate(dual, 0).resolve();
    assert!(runner.state().objects[&dual].tapped, "reach guard: tapped");
    runner.advance_until_stack_empty();
    // Two triggers would deal 2.
    assert_eq!(runner.life(P1), 19, "exactly one trigger");
}

/// CR 603.2 + the CR 602.2 non-mana gate: Immolation Shaman damages an
/// opponent who activates a non-mana ability of an artifact, creature, or land
/// (here a land), and not one who activates a mana ability.
#[test]
fn immolation_shaman_punishes_only_non_mana_activations() {
    let mut scenario = GameScenario::new();
    scenario.at_phase(Phase::PreCombatMain);
    scenario
        .add_creature(P0, "Immolation Shaman", 1, 1)
        .with_subtypes(vec!["Viashino", "Shaman"])
        .from_oracle_text(
            "Whenever an opponent activates an ability of an artifact, creature, or land that isn't a mana ability, this creature deals 1 damage to that player.\n{3}{R}{R}: This creature gets +3/+3 and gains menace until end of turn.",
        );
    let land = scenario
        .add_land_from_oracle(P1, "Scry Land", "{T}: Add {C}.\n{T}: Scry 1.")
        .id();
    let mut runner = scenario.build();
    {
        let state = runner.state_mut();
        state.active_player = P1;
        state.priority_player = P1;
        state.waiting_for = WaitingFor::Priority { player: P1 };
    }

    // Mana ability: no trigger.
    runner.activate(land, 0).resolve();
    assert_eq!(runner.life(P1), 20, "a mana ability does not trigger it");
    runner.state_mut().objects.get_mut(&land).unwrap().tapped = false;

    // Non-mana ability: 1 damage to the activating opponent.
    runner.activate(land, 1).resolve();
    runner.advance_until_stack_empty();
    assert_eq!(runner.life(P1), 19, "a non-mana land ability triggers it");
    assert_eq!(runner.life(P0), 20);
}
