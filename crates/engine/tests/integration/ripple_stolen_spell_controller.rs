//! CR 603.3a + CR 109.5: Ripple (CR 702.60a) is a "when you cast this spell"
//! trigger, so it is controlled by whoever controlled the spell when it was
//! cast. If Commandeer steals the Ripple spell before its trigger resolves, the
//! original caster still reveals from their own library and gets the free-cast
//! offer; the thief's library is untouched.

use engine::game::scenario::{GameScenario, P0, P1};
use engine::game::scenario_db::GameScenarioDbExt;
use engine::types::ability::{Effect, TargetRef};
use engine::types::actions::{CastChoice, GameAction};
use engine::types::game_state::{CastOfferKind, CastPaymentMode, StackEntryKind, WaitingFor};
use engine::types::identifiers::ObjectId;
use engine::types::mana::{ManaType, ManaUnit};
use engine::types::phase::Phase;
use engine::types::zones::Zone;

fn pool(mana: &[ManaType]) -> Vec<ManaUnit> {
    mana.iter()
        .map(|m| ManaUnit::new(*m, ObjectId(0), false, vec![]))
        .collect()
}

#[test]
fn stolen_ripple_spell_still_ripples_for_its_caster() {
    let db = crate::support::shared_card_db().expect("integration card fixture must load");
    let mut scenario = GameScenario::new();
    scenario.at_phase(Phase::PreCombatMain);
    let spell = scenario.add_real_card(P0, "Surging Flame", Zone::Hand, db);
    let p0_hit = scenario.add_real_card(P0, "Surging Flame", Zone::Library, db);
    let p0_filler = scenario.add_real_card(P0, "Mountain", Zone::Library, db);
    let commandeer = scenario.add_real_card(P1, "Commandeer", Zone::Hand, db);
    let p1_flame = scenario.add_real_card(P1, "Surging Flame", Zone::Library, db);
    let p1_filler = scenario.add_real_card(P1, "Mountain", Zone::Library, db);
    scenario.with_mana_pool(P0, pool(&[ManaType::Red, ManaType::Colorless]));
    scenario.with_mana_pool(
        P1,
        pool(&[
            ManaType::Blue,
            ManaType::Blue,
            ManaType::Colorless,
            ManaType::Colorless,
            ManaType::Colorless,
            ManaType::Colorless,
            ManaType::Colorless,
        ]),
    );
    let mut runner = scenario.build();
    engine::game::rehydrate_game_from_card_db(runner.state_mut(), db);
    for (player, library) in [
        (P0, vec![p0_hit, p0_filler]),
        (P1, vec![p1_flame, p1_filler]),
    ] {
        runner
            .state_mut()
            .players
            .iter_mut()
            .find(|p| p.id == player)
            .expect("player exists")
            .library = library.into();
    }

    {
        let _committed = runner.cast(spell).target_player(P1).commit();
    }
    let top = runner
        .state()
        .stack
        .last()
        .expect("ripple trigger on the stack");
    assert!(
        matches!(&top.kind, StackEntryKind::TriggeredAbility { ability, .. }
            if matches!(ability.effect, Effect::Ripple { .. }) && ability.source_id == spell),
        "reach guard: Surging Flame's ripple trigger is on top; stack = {:?}",
        runner.state().stack
    );

    runner.act(GameAction::PassPriority).expect("p0 pass");
    let card_id = runner.state().objects[&commandeer].card_id;
    runner
        .act(GameAction::CastSpell {
            object_id: commandeer,
            card_id,
            targets: vec![],
            payment_mode: CastPaymentMode::Auto,
        })
        .expect("P1 casts Commandeer");
    for _ in 0..8 {
        match &runner.state().waiting_for {
            WaitingFor::TargetSelection { .. } => {
                runner
                    .act(GameAction::ChooseTarget {
                        target: Some(TargetRef::Object(spell)),
                    })
                    .expect("target Surging Flame");
            }
            WaitingFor::ManaPayment { .. } => {
                runner.act(GameAction::PassPriority).expect("pay");
            }
            WaitingFor::Priority { .. } => break,
            other => panic!("unexpected Commandeer cast prompt: {other:?}"),
        }
    }
    for _ in 0..6 {
        if runner.state().objects[&commandeer].zone == Zone::Graveyard
            && matches!(runner.state().waiting_for, WaitingFor::Priority { .. })
        {
            break;
        }
        match &runner.state().waiting_for {
            WaitingFor::Priority { .. } => {
                runner.act(GameAction::PassPriority).expect("pass");
            }
            // "You may choose new targets for it": P1 keeps the targets.
            WaitingFor::OptionalEffectChoice { .. } => {
                runner
                    .act(GameAction::DecideOptionalEffect { accept: false })
                    .expect("keep the stolen spell's targets");
            }
            other => panic!("unexpected prompt while Commandeer resolves: {other:?}"),
        }
    }
    let state = runner.state();
    assert_eq!(
        state.objects[&commandeer].zone,
        Zone::Graveyard,
        "Commandeer resolved"
    );
    assert_eq!(
        state.objects[&spell].controller, P1,
        "reach guard: Commandeer gave P1 control of Surging Flame"
    );
    let trigger = state
        .stack
        .last()
        .expect("ripple trigger still on the stack");
    assert!(
        matches!(&trigger.kind, StackEntryKind::TriggeredAbility { ability, .. }
            if matches!(ability.effect, Effect::Ripple { .. }) && ability.controller == P0),
        "reach guard: the ripple trigger is still P0's; stack = {:?}",
        state.stack
    );

    // Both players pass; the ripple trigger resolves.
    for _ in 0..2 {
        if !matches!(runner.state().waiting_for, WaitingFor::Priority { .. }) {
            break;
        }
        runner.act(GameAction::PassPriority).expect("pass");
    }
    assert!(
        matches!(
            runner.state().waiting_for,
            WaitingFor::RippleRevealChoice { player, source_id, .. }
                if player == P0 && source_id == spell
        ),
        "CR 603.3a: the caster, not the thief, decides the reveal; waiting_for = {:?}",
        runner.state().waiting_for
    );
    runner
        .act(GameAction::RippleChoice {
            choice: CastChoice::Cast,
        })
        .expect("reveal");
    assert!(
        matches!(
            &runner.state().waiting_for,
            WaitingFor::CastOffer {
                player,
                kind: CastOfferKind::Ripple { hit_card, .. },
            } if *player == P0 && *hit_card == p0_hit
        ),
        "the caster is offered the same-named card from their own library; waiting_for = {:?}",
        runner.state().waiting_for
    );
    assert_eq!(runner.state().objects[&p1_flame].zone, Zone::Library);
    assert_eq!(
        runner.state().players[1].library.front().copied(),
        Some(p1_flame),
        "the thief's library is untouched"
    );
}
