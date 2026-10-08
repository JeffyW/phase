//! CR 113.6 + CR 113.6m + CR 603.2c: a spell's standalone "Whenever …" line is
//! a printed triggered ability that functions from the zone its own text names
//! (Killian's Confidence, Thunderblade Charge: the graveyard), not a delayed
//! trigger the spell creates on resolution.

use engine::game::combat::AttackTarget;
use engine::game::scenario::{GameRunner, GameScenario, P0, P1};
use engine::game::triggers::drain_order_triggers_with_identity;
use engine::types::actions::GameAction;
use engine::types::game_state::WaitingFor;
use engine::types::identifiers::ObjectId;
use engine::types::phase::Phase;
use engine::types::player::PlayerId;

const KILLIANS_CONFIDENCE: &str = "Target creature gets +1/+1 until end of turn. Draw a card.\nWhenever one or more creatures you control deal combat damage to a player, you may pay {W/B}. If you do, return this card from your graveyard to your hand.";
const THUNDERBLADE_CHARGE: &str = "Thunderblade Charge deals 3 damage to any target.\nWhenever one or more creatures you control deal combat damage to a player, if this card is in your graveyard, you may pay {2}{R}{R}{R}. If you do, you may cast it without paying its mana cost.";

const P2: PlayerId = PlayerId(2);

#[derive(Debug, Clone, Copy)]
enum CardZone {
    Graveyard,
    Hand,
}

/// Three players. P0's two attackers hit `defenders` (one attacker each), with
/// the printed spell in `zone`. Returns how many of its triggers are on the
/// stack after combat damage, before any resolves.
fn firings_after_combat(text: &str, zone: CardZone, defenders: &[PlayerId]) -> usize {
    let mut scenario = GameScenario::new_n_player(3, 9656);
    scenario.at_phase(Phase::PreCombatMain);
    let card = match zone {
        CardZone::Graveyard => scenario
            .add_spell_to_graveyard(P0, "Printed Spell", false)
            .from_oracle_text(text)
            .id(),
        CardZone::Hand => scenario
            .add_spell_to_hand_from_oracle(P0, "Printed Spell", false, text)
            .id(),
    };
    let attackers: Vec<ObjectId> = defenders
        .iter()
        .enumerate()
        .map(|(i, _)| {
            scenario
                .add_creature(P0, &format!("Attacker {i}"), 2, 2)
                .id()
        })
        .collect();
    let mut runner = scenario.build();
    drive_to_declare_attackers(&mut runner);
    let attacks: Vec<_> = attackers
        .iter()
        .zip(defenders)
        .map(|(&id, &defender)| (id, AttackTarget::Player(defender)))
        .collect();
    runner
        .declare_attackers(&attacks)
        .expect("declare attackers");
    for _ in 0..64 {
        let fired = runner
            .state()
            .stack
            .iter()
            .filter(|entry| entry.source_id == card)
            .count();
        if fired > 0 {
            return fired;
        }
        match runner.state().waiting_for.clone() {
            WaitingFor::OrderTriggers { .. } => {
                drain_order_triggers_with_identity(runner.state_mut());
            }
            WaitingFor::DeclareBlockers { .. } => {
                runner
                    .act(GameAction::DeclareBlockers {
                        assignments: vec![],
                    })
                    .expect("no blockers");
            }
            WaitingFor::Priority { .. } if runner.state().phase == Phase::PostCombatMain => {
                return 0;
            }
            WaitingFor::Priority { .. } => {
                runner.act(GameAction::PassPriority).expect("pass priority");
            }
            other => panic!("unexpected decision: {other:?}"),
        }
    }
    panic!("combat never finished");
}

fn drive_to_declare_attackers(runner: &mut GameRunner) {
    for _ in 0..16 {
        match runner.state().waiting_for.clone() {
            WaitingFor::DeclareAttackers { .. } => return,
            WaitingFor::Priority { .. } => {
                runner.act(GameAction::PassPriority).expect("pass priority");
            }
            other => panic!("unexpected decision: {other:?}"),
        }
    }
    panic!("never reached declare attackers");
}

/// CR 113.6m + CR 603.2c: Killian's Confidence functions from the graveyard
/// (its effect moves the card out of the graveyard) and triggers once per
/// player dealt combat damage, as a printed "one or more … to a player"
/// trigger. Two players hit → two firings; one player → one; the card in
/// hand → none.
#[test]
fn killians_confidence_triggers_from_the_graveyard_per_damaged_player() {
    assert_eq!(
        firings_after_combat(KILLIANS_CONFIDENCE, CardZone::Graveyard, &[P1, P2]),
        2,
        "P1 and P2 dealt combat damage"
    );
    assert_eq!(
        firings_after_combat(KILLIANS_CONFIDENCE, CardZone::Graveyard, &[P1]),
        1,
        "only P1 dealt combat damage"
    );
    assert_eq!(
        firings_after_combat(KILLIANS_CONFIDENCE, CardZone::Hand, &[P1, P2]),
        0,
        "the ability doesn't function from the hand"
    );
}

/// CR 113.6 + CR 603.4: Thunderblade Charge's "if this card is in your
/// graveyard" ability triggers from the graveyard, once per damaged player.
#[test]
fn thunderblade_charge_triggers_from_the_graveyard_per_damaged_player() {
    assert_eq!(
        firings_after_combat(THUNDERBLADE_CHARGE, CardZone::Graveyard, &[P1, P2]),
        2
    );
    assert_eq!(
        firings_after_combat(THUNDERBLADE_CHARGE, CardZone::Hand, &[P1, P2]),
        0
    );
}
