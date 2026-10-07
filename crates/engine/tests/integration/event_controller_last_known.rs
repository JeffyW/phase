//! CR 608.2h + CR 109.4 + CR 113.8: who "that permanent's controller" / "that
//! spell or ability's controller" is when an event's object has changed control
//! or left its zone before the trigger resolves.
//!
//! Oracle text is verbatim from Scryfall, except fixtures labelled synthetic.

use engine::game::scenario::{GameRunner, GameScenario, P0, P1};
use engine::game::triggers::drain_order_triggers_with_identity;
use engine::types::actions::GameAction;
use engine::types::game_state::WaitingFor;
use engine::types::identifiers::ObjectId;
use engine::types::mana::ManaCost;
use engine::types::phase::Phase;
use engine::types::player::PlayerId;
use engine::types::zones::Zone;

const GREMLIN_INFESTATION: &str = "Enchant artifact\nAt the beginning of your end step, this Aura deals 2 damage to enchanted artifact's controller.\nWhen enchanted artifact is put into a graveyard, create a 2/2 red Gremlin creature token.";
/// Synthetic: a one-line control change at instant speed.
const STEAL_ARTIFACT: &str = "Gain control of target artifact.";
/// Synthetic: an instant-speed bounce for an artifact.
const BOUNCE_ARTIFACT: &str = "Return target artifact to its owner's hand.";

fn free_spell(scenario: &mut GameScenario, owner: PlayerId, name: &str, text: &str) -> ObjectId {
    scenario
        .add_spell_to_hand_from_oracle(owner, name, true, text)
        .with_mana_cost(ManaCost::zero())
        .id()
}

fn give_priority(runner: &mut GameRunner, player: PlayerId) {
    let state = runner.state_mut();
    state.priority_player = player;
    state.waiting_for = WaitingFor::Priority { player };
}

/// Pass priority (resolving nothing) until `player` holds it.
fn priority_to(runner: &mut GameRunner, player: PlayerId) {
    let depth = runner.state().stack.len();
    for _ in 0..6 {
        match runner.state().waiting_for.clone() {
            WaitingFor::OrderTriggers { .. } => {
                drain_order_triggers_with_identity(runner.state_mut());
            }
            WaitingFor::Priority { player: p } if p == player => {
                assert_eq!(runner.state().stack.len(), depth, "nothing resolved");
                return;
            }
            WaitingFor::Priority { .. } => {
                runner.act(GameAction::PassPriority).expect("pass priority");
            }
            other => panic!("priority not reachable: {other:?}"),
        }
    }
    panic!("priority not reached");
}

/// Resolve the top stack object only.
fn resolve_one(runner: &mut GameRunner) {
    let depth = runner.state().stack.len();
    assert!(depth > 0, "something to resolve");
    for _ in 0..8 {
        if runner.state().stack.len() < depth {
            return;
        }
        match runner.state().waiting_for.clone() {
            WaitingFor::OrderTriggers { .. } => {
                drain_order_triggers_with_identity(runner.state_mut());
            }
            _ => {
                runner.act(GameAction::PassPriority).expect("pass priority");
            }
        }
    }
    panic!("the top object did not resolve");
}

fn life(runner: &GameRunner, player: PlayerId) -> i32 {
    runner.life(player)
}

/// Whether the enchanted artifact leaves before the end-step trigger resolves.
#[derive(Clone, Copy)]
enum HostDeparture {
    Bounced,
    Stays,
}

/// P1's Gremlin Infestation enchants P1's artifact, which P0 stole. At P1's end
/// step the Aura triggers; P0 optionally bounces the artifact in response.
/// Returns (P0, P1) life after the trigger resolves.
fn gremlin_board(departure: HostDeparture) -> (i32, i32) {
    let mut scenario = GameScenario::new();
    scenario.at_phase(Phase::PreCombatMain);
    let artifact = scenario.add_artifact_from_oracle(P1, "Ornament", "").id();
    let aura = scenario
        .add_enchantment_from_oracle(P1, "Gremlin Infestation", "")
        .with_subtypes(vec!["Aura"])
        .from_oracle_text(GREMLIN_INFESTATION)
        .id();
    let steal = free_spell(&mut scenario, P0, "Steal Artifact", STEAL_ARTIFACT);
    let bounce = free_spell(&mut scenario, P0, "Bounce Artifact", BOUNCE_ARTIFACT);
    let mut runner = scenario.build();
    {
        let state = runner.state_mut();
        state.objects.get_mut(&aura).unwrap().attached_to = Some(artifact.into());
        state
            .objects
            .get_mut(&artifact)
            .unwrap()
            .attachments
            .push(aura);
        state.layers_dirty.mark_full();
    }
    runner.cast(steal).target_object(artifact).resolve();
    assert_eq!(
        runner.state().objects[&artifact].controller,
        P0,
        "reach guard: P0 stole the artifact"
    );

    // P1's turn, its end step: the Aura's controller is P1 ("your end step").
    {
        let state = runner.state_mut();
        state.active_player = P1;
        state.phase = Phase::PostCombatMain;
    }
    give_priority(&mut runner, P1);
    runner.advance_to_phase(Phase::End);
    for _ in 0..4 {
        if !runner.state().stack.is_empty() {
            break;
        }
        if let WaitingFor::OrderTriggers { .. } = runner.state().waiting_for {
            drain_order_triggers_with_identity(runner.state_mut());
        } else {
            break;
        }
    }
    assert_eq!(
        runner.state().stack.len(),
        1,
        "reach guard: the end-step trigger is waiting"
    );

    if let HostDeparture::Bounced = departure {
        priority_to(&mut runner, P0);
        runner.cast(bounce).target_object(artifact).commit();
        resolve_one(&mut runner);
        assert_eq!(
            runner.state().objects[&artifact].zone,
            Zone::Hand,
            "reach guard: bounced to its owner"
        );
    }
    eprintln!(
        "DBG aura zone={:?} attached={:?} stack={} zc={:?} lki_art={:?}",
        runner.state().objects.get(&aura).map(|o| o.zone),
        runner
            .state()
            .objects
            .get(&aura)
            .and_then(|o| o.attached_to),
        runner.state().stack.len(),
        runner
            .state()
            .zone_changes_this_turn
            .iter()
            .map(|r| (r.object_id, r.from_zone, r.attached_to))
            .collect::<Vec<_>>(),
        runner
            .state()
            .lki_cache
            .get(&artifact)
            .map(|l| l.controller)
    );
    let before = (life(&runner, P0), life(&runner, P1));
    resolve_one(&mut runner);
    (life(&runner, P0) - before.0, life(&runner, P1) - before.1)
}

/// CR 608.2h + CR 303.4: "enchanted artifact's controller" when the artifact
/// left before the trigger resolved is its controller as it last existed on
/// the battlefield: P0, who stole it, not P1, who owns it.
#[test]
fn gremlin_infestation_damages_the_last_controller_of_a_departed_host() {
    assert_eq!(gremlin_board(HostDeparture::Bounced), (-2, 0));
}

/// Control: the stolen artifact stays, so its current controller (P0) is hit.
#[test]
fn gremlin_infestation_damages_the_controller_of_a_stolen_host_that_stays() {
    assert_eq!(gremlin_board(HostDeparture::Stays), (-2, 0));
}
