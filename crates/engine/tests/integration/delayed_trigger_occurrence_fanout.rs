//! CR 603.2c + CR 603.7b: a multi-fire delayed trigger ("whenever … this turn",
//! "until end of turn, whenever …") triggers on EACH occurrence in its event —
//! once per declared attacker or blocker — exactly as the same printed trigger
//! does. One fan-out authority (`occurrence_trigger_events`) serves printed and
//! delayed triggers; batched ("one or more") triggers fire once.
//!
//! Oracle text is verbatim from Scryfall, except fixtures labelled synthetic.

use engine::game::combat::AttackTarget;
use engine::game::scenario::{GameRunner, GameScenario, P0, P1};
use engine::game::triggers::drain_order_triggers_with_identity;
use engine::types::ability::TargetRef;
use engine::types::actions::GameAction;
use engine::types::counter::CounterType;
use engine::types::game_state::WaitingFor;
use engine::types::identifiers::ObjectId;
use engine::types::mana::{ManaColor, ManaCost};
use engine::types::phase::Phase;
use engine::types::player::PlayerId;
use engine::types::zones::Zone;

const SUMMON_LEVIATHAN: &str = "(As this Saga enters and after your draw step, add a lore counter. Sacrifice after III.)\n\
I \u{2014} Return each creature that isn't a Kraken, Leviathan, Merfolk, Octopus, or Serpent to its owner's hand.\n\
II, III \u{2014} Until end of turn, whenever a Kraken, Leviathan, Merfolk, Octopus, or Serpent attacks, draw a card.\n\
Ward {2}";
const STEADY_PROGRESS: &str = "Proliferate.\nDraw a card.";
const RIGHTEOUS_CAUSE: &str = "Whenever a creature attacks, you gain 1 life.";
const BATTLE_CRY: &str = "Untap all white creatures you control.\nWhenever a creature blocks this turn, it gets +0/+1 until end of turn.";
const CONSUMING_RAGE: &str = "Whenever a Minotaur attacks this turn, it gets +2/+0 until end of turn. Destroy that creature at end of combat.";
const GARRUK: &str = "Whenever a creature you control with power 4 or greater enters, draw a card.\n+2: Untap up to two target lands.\n\u{2212}3: Create a 4/4 green Beast creature token with trample.\n\u{2212}4: Until your next turn, whenever one or more creatures attack one of your opponents, those creatures get +2/+2 and gain trample until end of turn.";
const BASRI_KET: &str = "+1: Put a +1/+1 counter on up to one target creature. It gains indestructible until end of turn.\n\u{2212}2: Whenever one or more nontoken creatures attack this turn, create that many 1/1 white Soldier creature tokens that are tapped and attacking.\n\u{2212}6: You get an emblem with \"At the beginning of combat on your turn, create a 1/1 white Soldier creature token, then put a +1/+1 counter on each creature you control.\"";
const TASHA: &str = "+1: Until your next turn, whenever a creature attacks you or Tasha, Unholy Archmage, put a -1/-1 counter on that creature.\n\u{2212}2: Target opponent puts a creature card of their choice from their graveyard onto the battlefield under your control. That creature gains ward {2}.\n\u{2212}6: Target opponent reveals cards from the top of their library until they reveal three creature cards. Put those cards onto the battlefield under your control. That player puts the rest into their graveyard.";
const FIRST_DAY_OF_CLASS: &str = "Whenever a creature you control enters this turn, put a +1/+1 counter on it and it gains haste until end of turn.\nLearn. (You may reveal a Lesson card you own from outside the game and put it into your hand, or discard a card to draw a card.)";
const DONT_MOVE: &str = "Destroy all tapped creatures. Until your next turn, whenever a creature becomes tapped, destroy it.";
const SHRIVELING_ROT: &str = "Choose one \u{2014}\n\u{2022} Until end of turn, whenever a creature is dealt damage, destroy it.\n\u{2022} Until end of turn, whenever a creature dies, that creature's controller loses life equal to its toughness.\nEntwine {2}{B} (Choose both if you pay the entwine cost.)";
const FALSE_CURE: &str = "Until end of turn, whenever a player gains life, that player loses 2 life for each 1 life they gained.";
const MIRARI: &str = "(As this Saga enters and after your draw step, add a lore counter. Sacrifice after III.)\n\
I \u{2014} Return target instant card from your graveyard to your hand.\n\
II \u{2014} Return target sorcery card from your graveyard to your hand.\n\
III \u{2014} Until end of turn, whenever you cast an instant or sorcery spell, copy it. You may choose new targets for the copy.";
const UNSUMMON: &str = "Return target creature to its owner's hand.";
const PYROMANCER: &str = "{T}: This creature deals 1 damage to any target.";

fn board() -> GameScenario {
    let mut scenario = GameScenario::new();
    scenario.at_phase(Phase::PreCombatMain);
    let names_p0: Vec<String> = (0..10).map(|i| format!("P0 Card {i}")).collect();
    let names_p1: Vec<String> = (0..10).map(|i| format!("P1 Card {i}")).collect();
    scenario.with_library_top(P0, &names_p0.iter().map(String::as_str).collect::<Vec<_>>());
    scenario.with_library_top(P1, &names_p1.iter().map(String::as_str).collect::<Vec<_>>());
    scenario
}

fn typed(scenario: &mut GameScenario, owner: PlayerId, name: &str, subtype: &str) -> ObjectId {
    scenario
        .add_creature(owner, name, 2, 2)
        .with_subtypes(vec![subtype])
        .id()
}

fn free_spell(
    scenario: &mut GameScenario,
    owner: PlayerId,
    name: &str,
    instant: bool,
    text: &str,
) -> ObjectId {
    scenario
        .add_spell_to_hand_from_oracle(owner, name, instant, text)
        .with_mana_cost(ManaCost::zero())
        .id()
}

fn add_leviathan(scenario: &mut GameScenario, lore: u32) -> ObjectId {
    let saga = scenario
        .add_creature(P0, "Summon: Leviathan", 6, 6)
        .as_enchantment()
        .with_subtypes(vec!["Saga", "Leviathan"])
        .from_oracle_text_with_keywords(&["Ward"], SUMMON_LEVIATHAN)
        .id();
    scenario.with_counter(saga, CounterType::Lore, lore);
    saga
}

fn hand(runner: &GameRunner, player: PlayerId) -> usize {
    runner.state().players[player.0 as usize].hand.len()
}

fn zone(runner: &GameRunner, id: ObjectId) -> Zone {
    runner.state().objects[&id].zone
}

fn pt(runner: &mut GameRunner, id: ObjectId) -> (i32, i32) {
    runner.state_mut().layers_dirty.mark_full();
    engine::game::layers::evaluate_layers(runner.state_mut());
    let o = &runner.state().objects[&id];
    (o.power.unwrap_or(0), o.toughness.unwrap_or(0))
}

/// Resolve the stack, answering ordering, optional "you may" (accept), target
/// prompts (first legal) and proliferate choices (`proliferate_pick`). Stops at
/// the first other decision.
fn settle(runner: &mut GameRunner, proliferate_pick: &[ObjectId]) {
    for _ in 0..128 {
        match runner.state().waiting_for.clone() {
            WaitingFor::OrderTriggers { .. } => {
                drain_order_triggers_with_identity(runner.state_mut());
            }
            WaitingFor::OptionalEffectChoice { .. } => {
                runner
                    .act(GameAction::DecideOptionalEffect { accept: true })
                    .expect("accept the optional effect");
            }
            WaitingFor::LearnChoice { .. } => {
                runner
                    .act(GameAction::LearnDecision {
                        choice: engine::types::actions::LearnOption::Skip,
                    })
                    .expect("decline to learn");
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
    panic!("stack did not settle: {:?}", runner.state().waiting_for);
}

/// Pass through the game, declaring no attackers or blockers, until `stop`.
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
                runner.declare_attackers(&[]).expect("no attackers");
            }
            WaitingFor::DeclareBlockers { .. } => {
                runner
                    .act(GameAction::DeclareBlockers {
                        assignments: vec![],
                    })
                    .expect("no blockers");
            }
            WaitingFor::Priority { .. } => {
                runner.act(GameAction::PassPriority).expect("pass priority");
            }
            other => panic!("unexpected decision: {other:?}"),
        }
    }
    panic!(
        "never reached the stop condition: phase={:?} active={:?} waiting_for={:?}",
        runner.state().phase,
        runner.state().active_player,
        runner.state().waiting_for
    );
}

/// Declare `attackers` (each at `defender`) when the declaration is offered.
fn attack(runner: &mut GameRunner, attackers: &[ObjectId], defender: PlayerId) {
    let attacker_player = runner.state().objects[&attackers[0]].controller;
    drive_until(
        runner,
        |r| matches!(r.state().waiting_for, WaitingFor::DeclareAttackers { player, .. } if player == attacker_player),
    );
    let attacks: Vec<_> = attackers
        .iter()
        .map(|&id| (id, AttackTarget::Player(defender)))
        .collect();
    runner
        .declare_attackers(&attacks)
        .expect("declare attackers");
}

/// Declare `assignments` (blocker, attacker) when blocks are offered.
fn block(runner: &mut GameRunner, assignments: &[(ObjectId, ObjectId)]) {
    drive_until(runner, |r| {
        matches!(r.state().waiting_for, WaitingFor::DeclareBlockers { .. })
    });
    runner
        .act(GameAction::DeclareBlockers {
            assignments: assignments.to_vec(),
        })
        .expect("declare blockers");
}

/// Start P0's turn 2 at upkeep and advance into its precombat main, where
/// CR 714.3c adds a lore counter to each Saga; resolve the chapters.
fn advance_sagas(runner: &mut GameRunner) {
    {
        let state = runner.state_mut();
        state.turn_number = 2;
        state.active_player = P0;
        state.phase = Phase::Upkeep;
        state.priority_player = P0;
        state.waiting_for = WaitingFor::Priority { player: P0 };
    }
    runner.advance_to_phase(Phase::PreCombatMain);
    settle(runner, &[]);
}

/// CR 714.2b + CR 603.7b: the chapter-II delayed trigger names no controller.
/// It is installed on P1's turn — P0's Steady Progress proliferates the Saga
/// from lore 1 to 2 — and each of P1's two listed attackers (Octopus, Kraken)
/// draws P0 a card; P1's Bear attacking alongside them does not.
#[test]
fn leviathan_chapter_two_fires_for_an_opponents_listed_attacker() {
    let mut scenario = board();
    let saga = add_leviathan(&mut scenario, 1);
    let octopus = typed(&mut scenario, P1, "Octopus", "Octopus");
    let kraken = typed(&mut scenario, P1, "Kraken", "Kraken");
    let bear = typed(&mut scenario, P1, "Bear", "Bear");
    let progress = free_spell(&mut scenario, P0, "Steady Progress", true, STEADY_PROGRESS);
    let mut runner = scenario.build();
    {
        let state = runner.state_mut();
        state.active_player = P1;
        state.phase = Phase::PreCombatMain;
        state.priority_player = P1;
        state.waiting_for = WaitingFor::Priority { player: P1 };
    }
    runner
        .act(GameAction::PassPriority)
        .expect("P1 passes to P0");
    runner.cast(progress).commit();
    settle(&mut runner, &[saga]);
    assert_eq!(
        runner.state().objects[&saga].counters[&CounterType::Lore],
        2,
        "reach guard: proliferate added a lore counter"
    );
    assert_eq!(
        runner.state().delayed_triggers.len(),
        1,
        "reach guard: chapter II installed its delayed trigger on P1's turn"
    );

    // Baseline after Steady Progress's own draw.
    let p0_hand = hand(&runner, P0);
    attack(&mut runner, &[octopus, kraken, bear], P0);
    settle(&mut runner, &[]);
    assert_eq!(
        hand(&runner, P0),
        p0_hand + 2,
        "one draw each for the Octopus and the Kraken"
    );
}

/// CR 603.2c: two Leviathans each installed a chapter-II trigger. Two listed
/// attackers → four draws (the plain cardinality control; identical no-input
/// draws auto-order, so no prompt is required).
#[test]
fn two_leviathan_generators_fire_four_times() {
    let mut scenario = board();
    add_leviathan(&mut scenario, 1);
    add_leviathan(&mut scenario, 1);
    let merfolk = typed(&mut scenario, P0, "Merfolk", "Merfolk");
    let serpent = typed(&mut scenario, P0, "Serpent", "Serpent");
    let mut runner = scenario.build();
    advance_sagas(&mut runner);
    assert_eq!(
        runner.state().delayed_triggers.len(),
        2,
        "reach guard: two chapter-II generators installed"
    );
    let p0_hand = hand(&runner, P0);
    attack(&mut runner, &[merfolk, serpent], P1);
    settle(&mut runner, &[]);
    assert_eq!(
        hand(&runner, P0),
        p0_hand + 4,
        "two generators x two attackers"
    );
}

/// CR 603.3b: the ordering/resume witness. Two Leviathan generators and
/// Consuming Rage (a distinguishable delayed trigger) all trigger on two
/// listed Minotaur attackers, so the delayed batch needs a real ordering
/// prompt. After the order is submitted: four draws, each Minotaur pumped once,
/// and nothing re-fires when the game resumes. Righteous Cause (printed) gains
/// 1 life per attacker.
#[test]
fn two_leviathan_generators_fire_four_times_and_order_with_a_printed_trigger() {
    let mut scenario = board();
    add_leviathan(&mut scenario, 1);
    add_leviathan(&mut scenario, 1);
    scenario.add_enchantment_from_oracle(P0, "Righteous Cause", RIGHTEOUS_CAUSE);
    let merfolk = scenario
        .add_creature(P0, "Merfolk Minotaur", 2, 2)
        .with_subtypes(vec!["Merfolk", "Minotaur"])
        .id();
    let serpent = scenario
        .add_creature(P0, "Serpent Minotaur", 2, 2)
        .with_subtypes(vec!["Serpent", "Minotaur"])
        .id();
    let rage = free_spell(&mut scenario, P0, "Consuming Rage", false, CONSUMING_RAGE);
    let mut runner = scenario.build();
    advance_sagas(&mut runner);
    assert_eq!(
        runner.state().delayed_triggers.len(),
        2,
        "reach guard: two chapter-II generators installed"
    );
    runner.cast(rage).resolve();
    assert_eq!(
        runner.state().delayed_triggers.len(),
        3,
        "reach guard: Consuming Rage installed"
    );

    let (p0_hand, p0_life) = (hand(&runner, P0), runner.life(P0));
    attack(&mut runner, &[merfolk, serpent], P1);
    assert!(
        matches!(runner.state().waiting_for, WaitingFor::OrderTriggers { .. }),
        "a real ordering prompt: {:?}",
        runner.state().waiting_for
    );
    assert!(
        runner.state().pending_trigger_order.is_some(),
        "the order is pending while the prompt is open"
    );
    settle(&mut runner, &[]);
    assert_eq!(
        hand(&runner, P0),
        p0_hand + 4,
        "two generators x two attackers"
    );
    assert_eq!(runner.life(P0), p0_life + 2, "Righteous Cause per attacker");
    assert_eq!(pt(&mut runner, merfolk), (4, 2), "pumped once");
    assert_eq!(pt(&mut runner, serpent), (4, 2), "pumped once");

    drive_until(&mut runner, |r| r.state().phase == Phase::PostCombatMain);
    assert_eq!(
        hand(&runner, P0),
        p0_hand + 4,
        "no re-firing after the resume"
    );
}

/// Battle Cry: "it" is each blocker, and each of two blockers gets +0/+1.
#[test]
fn battle_cry_pumps_each_blocker() {
    let mut scenario = board();
    let a1 = scenario.add_creature(P0, "Attacker A", 2, 2).id();
    let a2 = scenario.add_creature(P0, "Attacker B", 2, 2).id();
    let b1 = scenario.add_creature(P1, "Blocker A", 1, 1).id();
    let b2 = scenario.add_creature(P1, "Blocker B", 1, 1).id();
    let cry = free_spell(&mut scenario, P0, "Battle Cry", true, BATTLE_CRY);
    let mut runner = scenario.build();
    runner.cast(cry).resolve();
    attack(&mut runner, &[a1, a2], P1);
    block(&mut runner, &[(b1, a1), (b2, a2)]);
    settle(&mut runner, &[]);
    assert_eq!(pt(&mut runner, b1), (1, 2), "Blocker A +0/+1");
    assert_eq!(pt(&mut runner, b2), (1, 2), "Blocker B +0/+1");
    assert_eq!(pt(&mut runner, a1), (2, 2), "attackers untouched");
}

/// Consuming Rage: each attacking Minotaur gets +2/+0, and each is destroyed at
/// end of combat.
#[test]
fn consuming_rage_pumps_and_destroys_each_minotaur() {
    let mut scenario = board();
    let m1 = typed(&mut scenario, P0, "Minotaur A", "Minotaur");
    let m2 = typed(&mut scenario, P0, "Minotaur B", "Minotaur");
    let bear = typed(&mut scenario, P0, "Bear", "Bear");
    let rage = free_spell(&mut scenario, P0, "Consuming Rage", false, CONSUMING_RAGE);
    let mut runner = scenario.build();
    runner.cast(rage).resolve();
    attack(&mut runner, &[m1, m2, bear], P1);
    settle(&mut runner, &[]);
    assert_eq!(pt(&mut runner, m1), (4, 2), "Minotaur A +2/+0");
    assert_eq!(pt(&mut runner, m2), (4, 2), "Minotaur B +2/+0");
    assert_eq!(pt(&mut runner, bear), (2, 2), "the Bear isn't a Minotaur");
    drive_until(&mut runner, |r| r.state().phase == Phase::PostCombatMain);
    assert_eq!(
        zone(&runner, m1),
        Zone::Graveyard,
        "destroyed at end of combat"
    );
    assert_eq!(
        zone(&runner, m2),
        Zone::Graveyard,
        "destroyed at end of combat"
    );
    assert_eq!(zone(&runner, bear), Zone::Battlefield);
}

/// Hostile: one Minotaur is bounced in response to its trigger. The other
/// Minotaur's own firing still pumps it; the bounced one's firing affects
/// nothing.
#[test]
fn consuming_rage_survives_one_minotaur_leaving_before_its_trigger_resolves() {
    let mut scenario = board();
    let m1 = typed(&mut scenario, P0, "Minotaur A", "Minotaur");
    let m2 = typed(&mut scenario, P0, "Minotaur B", "Minotaur");
    let rage = free_spell(&mut scenario, P0, "Consuming Rage", false, CONSUMING_RAGE);
    let unsummon = free_spell(&mut scenario, P0, "Unsummon", true, UNSUMMON);
    let mut runner = scenario.build();
    runner.cast(rage).resolve();
    attack(&mut runner, &[m1, m2], P1);
    if matches!(runner.state().waiting_for, WaitingFor::OrderTriggers { .. }) {
        drain_order_triggers_with_identity(runner.state_mut());
    }
    assert_eq!(
        runner.state().stack.len(),
        2,
        "reach guard: one firing per Minotaur"
    );
    runner.cast(unsummon).target_object(m1).commit();
    settle(&mut runner, &[]);
    assert_eq!(zone(&runner, m1), Zone::Hand, "reach guard: bounced");
    assert_eq!(
        pt(&mut runner, m2),
        (4, 2),
        "the remaining Minotaur is pumped"
    );
}

/// Garruk −4 (batched, "one or more"): one firing for the whole declaration;
/// each attacker gets +2/+2 exactly once.
#[test]
fn garruk_batched_attack_trigger_fires_once() {
    let mut scenario = board();
    let garruk = scenario
        .add_planeswalker_from_oracle(P0, "Garruk, Curse Breaker", "Garruk", 5, GARRUK)
        .as_legendary()
        .id();
    let a1 = scenario.add_creature(P0, "Attacker A", 2, 2).id();
    let a2 = scenario.add_creature(P0, "Attacker B", 2, 2).id();
    let mut runner = scenario.build();
    runner.activate(garruk, 2).resolve();
    assert_eq!(runner.state().delayed_triggers.len(), 1, "reach guard");
    attack(&mut runner, &[a1, a2], P1);
    if matches!(runner.state().waiting_for, WaitingFor::OrderTriggers { .. }) {
        drain_order_triggers_with_identity(runner.state_mut());
    }
    assert_eq!(
        runner.state().stack.len(),
        1,
        "one firing for the whole declaration"
    );
    settle(&mut runner, &[]);
    assert_eq!(pt(&mut runner, a1), (4, 4), "+2/+2 once");
    assert_eq!(pt(&mut runner, a2), (4, 4), "+2/+2 once");
    for (id, name) in [(a1, "A"), (a2, "B")] {
        assert!(
            runner.state().objects[&id]
                .keywords
                .contains(&engine::types::keywords::Keyword::Trample),
            "Attacker {name} gained trample"
        );
    }
}

/// Batched ("one or more") delayed control with a non-additive body: one
/// firing for the whole declaration, so one draw for two attackers. Synthetic.
#[test]
fn batched_delayed_attack_trigger_draws_once() {
    let mut scenario = board();
    let a1 = scenario.add_creature(P0, "Attacker A", 2, 2).id();
    let a2 = scenario.add_creature(P0, "Attacker B", 2, 2).id();
    let spell = free_spell(
        &mut scenario,
        P0,
        "Synthetic Muster",
        false,
        "Until end of turn, whenever one or more creatures attack, draw a card.",
    );
    let mut runner = scenario.build();
    runner.cast(spell).resolve();
    assert_eq!(runner.state().delayed_triggers.len(), 1, "reach guard");
    let p0_hand = hand(&runner, P0);
    attack(&mut runner, &[a1, a2], P1);
    settle(&mut runner, &[]);
    assert_eq!(hand(&runner, P0), p0_hand + 1, "one firing, one draw");
}

/// Basri Ket −2 (batched): two nontoken attackers and one token attacker →
/// one firing creating two Soldiers, which enter attacking and don't re-fire
/// (CR 508.3a: they were never declared as attackers).
#[test]
fn basri_ket_batched_attack_trigger_counts_nontoken_attackers_once() {
    let mut scenario = board();
    let basri = scenario
        .add_planeswalker_from_oracle(P0, "Basri Ket", "Basri", 5, BASRI_KET)
        .as_legendary()
        .id();
    let a1 = scenario.add_creature(P0, "Attacker A", 2, 2).id();
    let a2 = scenario.add_creature(P0, "Attacker B", 2, 2).id();
    let token = scenario.add_creature(P0, "Token Attacker", 1, 1).id();
    let mut runner = scenario.build();
    runner.state_mut().objects.get_mut(&token).unwrap().is_token = true;
    runner.activate(basri, 1).resolve();
    let soldiers = |r: &GameRunner| {
        r.state()
            .objects
            .values()
            .filter(|o| o.zone == Zone::Battlefield && o.name == "Soldier")
            .count()
    };
    attack(&mut runner, &[a1, a2, token], P1);
    settle(&mut runner, &[]);
    assert_eq!(soldiers(&runner), 2, "that many = two nontoken attackers");
    drive_until(&mut runner, |r| r.state().phase == Phase::PostCombatMain);
    assert_eq!(soldiers(&runner), 2, "the Soldiers don't re-fire it");
}

/// CR 509.3c (bare form): one firing per blocked attacker, however many
/// creatures block it. Two blockers on one attacker and one on another → each
/// attacker gets exactly one +1/+1. Synthetic delayed text.
#[test]
fn bare_becomes_blocked_fires_once_per_attacker() {
    let mut scenario = board();
    let attacker = scenario.add_creature(P0, "Attacker", 2, 2).id();
    let second = scenario.add_creature(P0, "Second Attacker", 2, 2).id();
    let b1 = scenario.add_creature(P1, "Blocker A", 1, 1).id();
    let b2 = scenario.add_creature(P1, "Blocker B", 1, 1).id();
    let b3 = scenario.add_creature(P1, "Blocker C", 1, 1).id();
    let spell = free_spell(
        &mut scenario,
        P0,
        "Synthetic Rally",
        false,
        "Until end of turn, whenever a creature becomes blocked, it gets +1/+1 until end of turn.",
    );
    let mut runner = scenario.build();
    runner.cast(spell).resolve();
    attack(&mut runner, &[attacker, second], P1);
    block(&mut runner, &[(b1, attacker), (b2, attacker), (b3, second)]);
    settle(&mut runner, &[]);
    assert_eq!(pt(&mut runner, attacker), (3, 3), "exactly one +1/+1");
    assert_eq!(pt(&mut runner, second), (3, 3), "exactly one +1/+1");
}

/// CR 509.3d (qualified form): one firing for each matching blocker, binding
/// that blocker. Synthetic delayed text shaped like Zombie Boa's (whose own
/// "creature of that color" qualifier is not parsed; no Boa support claimed).
#[test]
fn qualified_becomes_blocked_fires_once_per_matching_blocker() {
    let mut scenario = board();
    let attacker = scenario.add_creature(P0, "Attacker", 5, 5).id();
    let w1 = scenario
        .add_creature(P1, "White A", 1, 1)
        .with_color(vec![ManaColor::White])
        .id();
    let w2 = scenario
        .add_creature(P1, "White B", 1, 1)
        .with_color(vec![ManaColor::White])
        .id();
    let red = scenario
        .add_creature(P1, "Red", 1, 1)
        .with_color(vec![ManaColor::Red])
        .id();
    let spell = free_spell(
        &mut scenario,
        P0,
        "Synthetic Boa Ward",
        false,
        "Until end of turn, whenever a creature becomes blocked by a white creature, destroy that creature.",
    );
    let mut runner = scenario.build();
    runner.cast(spell).resolve();
    attack(&mut runner, &[attacker], P1);
    block(
        &mut runner,
        &[(w1, attacker), (w2, attacker), (red, attacker)],
    );
    settle(&mut runner, &[]);
    assert_eq!(zone(&runner, w1), Zone::Graveyard);
    assert_eq!(zone(&runner, w2), Zone::Graveyard);
    assert_eq!(zone(&runner, red), Zone::Battlefield, "not white");
    assert_eq!(
        zone(&runner, attacker),
        Zone::Battlefield,
        "not the attacker"
    );
}

/// Tasha +1: each creature attacking P0 gets a -1/-1 counter (on P1's turn,
/// within "until your next turn").
#[test]
fn tasha_puts_a_counter_on_each_attacker() {
    let mut scenario = board();
    let tasha = scenario
        .add_planeswalker_from_oracle(P0, "Tasha, Unholy Archmage", "Tasha", 4, TASHA)
        .as_legendary()
        .id();
    let a1 = scenario.add_creature(P1, "Attacker A", 3, 3).id();
    let a2 = scenario.add_creature(P1, "Attacker B", 3, 3).id();
    let mut runner = scenario.build();
    runner.activate(tasha, 0).resolve();
    attack(&mut runner, &[a1, a2], P0);
    settle(&mut runner, &[]);
    for (id, name) in [(a1, "A"), (a2, "B")] {
        assert_eq!(
            runner.state().objects[&id]
                .counters
                .get(&CounterType::Minus1Minus1),
            Some(&1),
            "Attacker {name} has one -1/-1 counter"
        );
    }
}

/// First Day of Class: each creature that enters gets the counter and haste.
#[test]
fn first_day_of_class_marks_each_entering_creature() {
    let mut scenario = board();
    let class = free_spell(
        &mut scenario,
        P0,
        "First Day of Class",
        true,
        FIRST_DAY_OF_CLASS,
    );
    let t1 = free_spell(
        &mut scenario,
        P0,
        "Token A",
        true,
        "Create a 1/1 green Saproling creature token.",
    );
    let t2 = free_spell(
        &mut scenario,
        P0,
        "Token B",
        true,
        "Create a 1/1 green Saproling creature token.",
    );
    let mut runner = scenario.build();
    runner.cast(class).resolve();
    settle(&mut runner, &[]);
    runner.cast(t1).resolve();
    settle(&mut runner, &[]);
    runner.cast(t2).resolve();
    settle(&mut runner, &[]);
    let saprolings: Vec<ObjectId> = runner
        .state()
        .objects
        .values()
        .filter(|o| o.zone == Zone::Battlefield && o.name == "Saproling")
        .map(|o| o.id)
        .collect();
    assert_eq!(saprolings.len(), 2, "reach guard");
    for id in saprolings {
        assert_eq!(
            pt(&mut runner, id),
            (2, 2),
            "each Saproling has the counter"
        );
        assert!(
            runner.state().objects[&id]
                .keywords
                .contains(&engine::types::keywords::Keyword::Haste),
            "each Saproling has haste"
        );
    }
}

/// Don't Move: a creature that becomes tapped is destroyed (it, not the spell).
#[test]
fn dont_move_destroys_the_creature_that_becomes_tapped() {
    let mut scenario = board();
    let victim = scenario.add_creature(P1, "Victim", 2, 2).id();
    let bystander = scenario.add_creature(P1, "Bystander", 2, 2).id();
    let dont_move = free_spell(&mut scenario, P0, "Don't Move", false, DONT_MOVE);
    let tap = free_spell(&mut scenario, P0, "Tap It", true, "Tap target creature.");
    let mut runner = scenario.build();
    runner.cast(dont_move).resolve();
    runner.cast(tap).target_object(victim).resolve();
    settle(&mut runner, &[]);
    assert_eq!(zone(&runner, victim), Zone::Graveyard);
    assert_eq!(zone(&runner, bystander), Zone::Battlefield);
}

/// Shriveling Rot (first mode): the creature dealt damage is destroyed, not the
/// creature that dealt it (CR 120.1: "it" names the recipient).
#[test]
fn shriveling_rot_destroys_the_damaged_creature_not_the_dealer() {
    let mut scenario = board();
    let pyromancer = scenario
        .add_creature_from_oracle(P0, "Prodigal Pyromancer", 1, 1, PYROMANCER)
        .id();
    let victim = scenario.add_creature(P1, "Victim", 3, 3).id();
    let rot = free_spell(&mut scenario, P0, "Shriveling Rot", true, SHRIVELING_ROT);
    let mut runner = scenario.build();
    runner.cast(rot).modes(&[0]).resolve();
    runner
        .activate(pyromancer, 0)
        .target_object(victim)
        .resolve();
    settle(&mut runner, &[]);
    assert_eq!(
        zone(&runner, victim),
        Zone::Graveyard,
        "the damaged creature"
    );
    assert_eq!(
        zone(&runner, pyromancer),
        Zone::Battlefield,
        "not the dealer"
    );
}

/// CR 120.4b + CR 608.2c: a batched ("one or more") damage trigger's "that
/// much" is the damage dealt, not the number of creatures dealt damage. A
/// Lightning Bolt dealing 3 to one creature gains 3 life, from both a delayed
/// trigger and the same printed trigger (synthetic text). The matching-subject
/// count stays a headcount where the event has no magnitude (the Basri Ket row).
#[test]
fn batched_damage_trigger_that_much_reads_the_damage_not_the_headcount() {
    const DELAYED: &str = "Until end of turn, whenever one or more creatures you control are dealt damage, you gain that much life.";
    const PRINTED: &str =
        "Whenever one or more creatures you control are dealt damage, you gain that much life.";
    const BOLT: &str = "Lightning Bolt deals 3 damage to any target.";
    for printed in [false, true] {
        let label = if printed { "printed" } else { "delayed" };
        let mut scenario = board();
        let wall = scenario.add_creature(P0, "Sturdy Wall", 2, 5).id();
        let spell = if printed {
            scenario.add_enchantment_from_oracle(P0, "Synthetic Mending", PRINTED);
            None
        } else {
            Some(free_spell(
                &mut scenario,
                P0,
                "Synthetic Mending",
                true,
                DELAYED,
            ))
        };
        let bolt = free_spell(&mut scenario, P0, "Lightning Bolt", true, BOLT);
        let mut runner = scenario.build();
        if let Some(spell) = spell {
            runner.cast(spell).resolve();
            assert_eq!(
                runner.state().delayed_triggers.len(),
                1,
                "[{label}] reach guard"
            );
        }
        let life = runner.life(P0);
        runner.cast(bolt).target_object(wall).resolve();
        settle(&mut runner, &[]);
        assert_eq!(
            runner.state().objects[&wall].damage_marked,
            3,
            "[{label}] reach guard: the Bolt dealt 3"
        );
        assert_eq!(runner.life(P0), life + 3, "[{label}] that much = 3 damage");
    }
}

/// False Cure: "that player" is the player who gained life — P1 loses life and
/// P0, the caster, doesn't. (The amount is out of scope here: "2 life for each
/// 1 life they gained" currently parses as a fixed 2, disclosed separately.)
#[test]
fn false_cure_punishes_the_player_who_gains_life() {
    let mut scenario = board();
    let cure = free_spell(&mut scenario, P0, "False Cure", true, FALSE_CURE);
    let gift = free_spell(
        &mut scenario,
        P0,
        "Gift",
        true,
        "Target player gains 2 life.",
    );
    let mut runner = scenario.build();
    runner.cast(cure).resolve();
    let gift_outcome = runner.cast(gift).target_player(P1).resolve();
    assert!(
        gift_outcome.events().iter().any(|e| matches!(
            e,
            engine::types::events::GameEvent::LifeChanged { player_id, amount: 2, .. }
                if *player_id == P1
        )),
        "reach guard: P1 gained 2 life"
    );
    settle(&mut runner, &[]);
    assert!(runner.life(P1) < 22, "the gaining player loses life");
    assert_eq!(runner.life(P0), 20, "the caster doesn't");
}

/// The Mirari Conjecture III: each instant cast is copied once.
#[test]
fn mirari_conjecture_copies_each_instant_once() {
    let mut scenario = board();
    let saga = scenario
        .add_enchantment_from_oracle(P0, "The Mirari Conjecture", "")
        .with_subtypes(vec!["Saga"])
        .from_oracle_text(MIRARI)
        .id();
    scenario.with_counter(saga, CounterType::Lore, 2);
    let gain = free_spell(&mut scenario, P0, "Gain", true, "You gain 1 life.");
    let mut runner = scenario.build();
    advance_sagas(&mut runner);
    assert_eq!(
        runner.state().delayed_triggers.len(),
        1,
        "reach guard: chapter III"
    );
    let life = runner.life(P0);
    runner.cast(gain).commit();
    settle(&mut runner, &[]);
    assert_eq!(runner.life(P0), life + 2, "the spell and one copy");
}
