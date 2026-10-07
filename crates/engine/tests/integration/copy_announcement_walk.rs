//! CR 601.2c + CR 707.12 + CR 115.6: announcing the targets of a freshly cast
//! copy (`WaitingFor::CopyRetarget` in Announce mode). The walk is the
//! production casting walk: an optional slot ("up to N") may be declined, a
//! slot "of an opponent's choice" is announced by its chooser while the copy
//! stays its caster's, and a copy whose spell has no ability announces nothing
//! and is still cast.
//!
//! Oracle text is verbatim from Scryfall (Mizzix's Mastery, Frost Breath,
//! Volcanic Offering, Baron Helmut Zemo).

use engine::game::scenario::{GameRunner, GameScenario, P0, P1};
use engine::types::ability::{
    AbilityDefinition, AbilityKind, Effect, EffectKind, QuantityExpr, TargetFilter, TargetRef,
};
use engine::types::actions::GameAction;
use engine::types::events::GameEvent;
use engine::types::game_state::{CastPaymentMode, CopyChoiceMode, WaitingFor};
use engine::types::identifiers::ObjectId;
use engine::types::mana::{ManaCost, ManaCostShard};
use engine::types::phase::Phase;
use engine::types::player::PlayerId;
use engine::types::zones::Zone;

const MIZZIXS_MASTERY: &str = "Exile target card that's an instant or sorcery from your graveyard. For each card exiled this way, copy it, and you may cast the copy without paying its mana cost. Exile Mizzix's Mastery.\nOverload {5}{R}{R}{R} (You may cast this spell for its overload cost. If you do, change \"target\" in its text to \"each.\")";
const FROST_BREATH: &str = "Tap up to two target creatures. Those creatures don't untap during their controller's next untap step.";
const VOLCANIC_OFFERING: &str = "Destroy target nonbasic land you don't control and target nonbasic land of an opponent's choice you don't control.\nVolcanic Offering deals 7 damage to target creature you don't control and 7 damage to target creature of an opponent's choice you don't control.";
const BARON_HELMUT_ZEMO: &str = "Whenever you cast a black spell from your hand, Baron Helmut Zemo connives.\nBoast \u{2014} Exile any number of black cards from your graveyard with fifteen or more black mana symbols among their mana costs: Copy those exiled cards. You may cast up to three of the copies without paying their mana costs. (Activate only if this creature attacked this turn and only once each turn.)";

/// Pass priority until a non-priority prompt opens.
fn pass_to_choice(r: &mut GameRunner) {
    for _ in 0..32 {
        if !matches!(r.state().waiting_for, WaitingFor::Priority { .. }) {
            return;
        }
        assert!(!r.state().stack.is_empty(), "reach: the stack drained");
        r.act(GameAction::PassPriority).expect("pass");
    }
    panic!("no choice opened");
}

/// Cast Mizzix's Mastery at `source` (an instant/sorcery in P0's graveyard),
/// choose to cast its copy, and return the copy's stack id at its first
/// announcement prompt.
fn mastery_copy(mut scenario: GameScenario, source: ObjectId) -> (GameRunner, ObjectId, ObjectId) {
    let mastery = scenario
        .add_spell_to_hand(P0, "Mizzix's Mastery", false)
        .from_oracle_text_with_keywords(&["Overload"], MIZZIXS_MASTERY)
        .with_mana_cost(ManaCost::zero())
        .id();
    let mut r = scenario.build();
    r.cast(mastery).target_object(source).commit();
    pass_to_choice(&mut r);
    assert!(
        matches!(
            r.state().waiting_for,
            WaitingFor::ChooseFromZoneChoice { .. }
        ),
        "reach: Mizzix offers the copy, got {:?}",
        r.state().waiting_for
    );
    r.act(GameAction::SelectCards {
        cards: vec![source],
    })
    .expect("cast the copy");
    let WaitingFor::CopyRetarget { copy_id, mode, .. } = r.state().waiting_for else {
        panic!(
            "expected the copy's announcement, got {:?}",
            r.state().waiting_for
        );
    };
    assert_eq!(
        mode,
        Some(CopyChoiceMode::Announce),
        "reach: an announcement"
    );
    (r, mastery, copy_id)
}

fn begin_cast(r: &mut GameRunner, spell: ObjectId) {
    r.act(GameAction::CastSpell {
        object_id: spell,
        card_id: r.state().objects[&spell].card_id,
        targets: vec![],
        payment_mode: CastPaymentMode::Auto,
    })
    .expect("cast begins");
}

fn declared(r: &GameRunner, entry: ObjectId) -> Vec<TargetRef> {
    let ability = r
        .state()
        .stack
        .iter()
        .find(|e| e.id == entry)
        .and_then(|e| e.ability())
        .expect("the announced spell is on the stack");
    engine::game::ability_utils::declared_targets_in_chain(ability)
}

fn resolve_entry(r: &mut GameRunner, entry: ObjectId) {
    for _ in 0..32 {
        if !r.state().stack.iter().any(|e| e.id == entry) {
            return;
        }
        r.act(GameAction::PassPriority).expect("resolve");
    }
    panic!("the spell did not resolve");
}

/// The engine-derived permissions of the current announcement slot:
/// `(can_decline, can_keep, can_keep_rest)`.
fn permissions(r: &GameRunner) -> (bool, bool, bool) {
    match &r.state().waiting_for {
        WaitingFor::CopyRetarget {
            target_slots,
            current_slot,
            can_keep_rest,
            ..
        } => (
            target_slots[*current_slot].can_decline,
            target_slots[*current_slot].can_keep,
            *can_keep_rest,
        ),
        other => panic!("expected the copy announcement, got {other:?}"),
    }
}

/// Frost Breath with `first` creatures chosen (0 or 1), then a decline.
/// `copy`: through Mizzix's Mastery, else the ordinary cast (control).
fn frost_breath(copy: bool, first: bool) {
    let mut s = GameScenario::new();
    s.at_phase(Phase::PreCombatMain);
    let a = s.add_creature(P1, "Creature A", 2, 9).id();
    let b = s.add_creature(P1, "Creature B", 2, 9).id();
    let (mut r, mastery, entry) = if copy {
        let frost = s
            .add_spell_to_graveyard(P0, "Frost Breath", true)
            .from_oracle_text(FROST_BREATH)
            .id();
        let (r, mastery, copy_id) = mastery_copy(s, frost);
        (r, Some(mastery), copy_id)
    } else {
        let frost = s
            .add_spell_to_hand_from_oracle(P0, "Frost Breath", true, FROST_BREATH)
            .with_mana_cost(ManaCost::zero())
            .id();
        let mut r = s.build();
        begin_cast(&mut r, frost);
        (r, None, frost)
    };
    if copy {
        assert_eq!(
            permissions(&r),
            (true, false, false),
            "the optional slot may be declined; there is nothing to keep"
        );
    }
    if first {
        r.act(GameAction::ChooseTarget {
            target: Some(TargetRef::Object(a)),
        })
        .expect("choose A");
        if copy {
            assert!(
                permissions(&r).0,
                "the second optional slot may be declined"
            );
        }
    }
    r.act(GameAction::ChooseTarget { target: None })
        .expect("CR 115.6: declining the optional slot is accepted");
    assert!(
        !matches!(
            r.state().waiting_for,
            WaitingFor::CopyRetarget { .. } | WaitingFor::TargetSelection { .. }
        ),
        "declining ends the \"up to two\" announcement"
    );
    let announced = declared(&r, entry);
    let expected: Vec<TargetRef> = if first {
        vec![TargetRef::Object(a)]
    } else {
        Vec::new()
    };
    assert_eq!(announced, expected);
    resolve_entry(&mut r, entry);
    assert_eq!(r.state().objects[&a].tapped, first);
    assert!(!r.state().objects[&b].tapped);
    if let Some(mastery) = mastery {
        resolve_entry(&mut r, mastery);
        assert_eq!(r.state().objects[&mastery].zone, Zone::Exile);
    }
}

#[test]
fn copy_announcement_declines_an_optional_slot_with_no_target() {
    frost_breath(true, false);
}

#[test]
fn copy_announcement_declines_after_one_target() {
    frost_breath(true, true);
}

#[test]
fn ordinary_frost_breath_decline_controls() {
    frost_breath(false, false);
    frost_breath(false, true);
}

/// Volcanic Offering: slots 1 and 3 are "of an opponent's choice". Asserts the
/// player each announcement prompt asks, and the copy's resolved board.
fn volcanic_offering(copy: bool) {
    let mut s = GameScenario::new();
    s.at_phase(Phase::PreCombatMain);
    let l1 = s.add_land_from_oracle(P1, "Opponent Nonbasic A", "").id();
    let l2 = s.add_land_from_oracle(P1, "Opponent Nonbasic B", "").id();
    let c1 = s.add_creature(P1, "Opponent Creature A", 3, 12).id();
    let c2 = s.add_creature(P1, "Opponent Creature B", 3, 12).id();
    let (mut r, mastery, entry) = if copy {
        let offering = s
            .add_spell_to_graveyard(P0, "Volcanic Offering", true)
            .from_oracle_text(VOLCANIC_OFFERING)
            .id();
        let (r, mastery, copy_id) = mastery_copy(s, offering);
        (r, Some(mastery), copy_id)
    } else {
        let offering = s
            .add_spell_to_hand_from_oracle(P0, "Volcanic Offering", true, VOLCANIC_OFFERING)
            .with_mana_cost(ManaCost::zero())
            .id();
        let mut r = s.build();
        begin_cast(&mut r, offering);
        (r, None, offering)
    };
    let mut askers: Vec<PlayerId> = Vec::new();
    for target in [l1, l2, c1, c2] {
        let asker = match &r.state().waiting_for {
            WaitingFor::CopyRetarget {
                player, controller, ..
            } => {
                assert_eq!(
                    controller.unwrap_or(*player),
                    P0,
                    "the copy stays its caster's"
                );
                *player
            }
            WaitingFor::TargetSelection { player, .. } => *player,
            other => panic!("expected an announcement prompt, got {other:?}"),
        };
        askers.push(asker);
        r.act(GameAction::ChooseTarget {
            target: Some(TargetRef::Object(target)),
        })
        .expect("the asked player announces");
    }
    assert_eq!(
        askers,
        vec![P0, P1, P0, P1],
        "CR 601.2c: each slot is announced by its chooser"
    );
    let controller = r
        .state()
        .stack
        .iter()
        .find(|e| e.id == entry)
        .map(|e| e.controller)
        .expect("the announced spell is on the stack");
    assert_eq!(controller, P0, "CR 112.2: the caster controls the spell");
    if copy {
        assert!(
            matches!(r.state().waiting_for, WaitingFor::Priority { player } if player == P0),
            "priority returns to the copy's controller, got {:?}",
            r.state().waiting_for
        );
    }
    resolve_entry(&mut r, entry);
    assert_eq!(
        [c1, c2].map(|c| r.state().objects[&c].damage_marked),
        [7, 7]
    );
    assert!([l1, l2]
        .iter()
        .all(|l| r.state().objects[l].zone == Zone::Graveyard));
    if let Some(mastery) = mastery {
        resolve_entry(&mut r, mastery);
        assert_eq!(r.state().objects[&mastery].zone, Zone::Exile);
        assert_eq!(r.state().objects[&mastery].controller, P0);
    }
}

#[test]
fn copy_announcement_routes_each_slot_to_its_chooser() {
    volcanic_offering(true);
}

#[test]
fn ordinary_volcanic_offering_chooser_control() {
    volcanic_offering(false);
}

/// Baron Helmut Zemo's Boast copies two exiled cards: `first` is a vanilla
/// creature card (no spell ability) or, as the control, an instant. Both
/// copies are cast, and the batch completes.
fn zemo_batch(vanilla_first: bool) {
    let mut s = GameScenario::new();
    s.at_phase(Phase::PostCombatMain);
    s.with_library_top(P0, &["Draw A", "Draw B", "Draw C", "Draw D"]);
    let zemo = s
        .add_creature_from_oracle(P0, "Baron Helmut Zemo", 3, 3, BARON_HELMUT_ZEMO)
        .id();
    // Fixture: fifteen black symbols per card meets the Boast cost.
    let black = ManaCost::Cost {
        shards: vec![ManaCostShard::Black; 15],
        generic: 0,
    };
    let draw = AbilityDefinition::new(
        AbilityKind::Spell,
        Effect::Draw {
            count: QuantityExpr::Fixed { value: 1 },
            target: TargetFilter::Controller,
        },
    );
    let first = if vanilla_first {
        s.add_creature_to_graveyard(P0, "Walking Corpse", 2, 2)
            .from_oracle_text("")
            .with_mana_cost(black.clone())
            .id()
    } else {
        s.add_spell_to_graveyard(P0, "Draw Instant A", true)
            .with_ability_definition(draw.clone())
            .with_mana_cost(black.clone())
            .id()
    };
    let second = s
        .add_spell_to_graveyard(P0, "Draw Instant B", true)
        .with_ability_definition(draw)
        .with_mana_cost(black)
        .id();
    let mut r = s.build();
    if vanilla_first {
        assert!(
            r.state().objects[&first].abilities.is_empty(),
            "reach: the vanilla creature has no spell ability"
        );
    }
    r.state_mut().creatures_attacked_this_turn.insert(zemo);
    let index = r.state().objects[&zemo]
        .abilities
        .iter()
        .position(|a| matches!(a.effect.as_ref(), Effect::CastCopyOfCard { .. }))
        .expect("reach: Zemo's Boast");
    r.act(GameAction::ActivateAbility {
        source_id: zemo,
        ability_index: index,
    })
    .expect("activate Boast");
    r.act(GameAction::SelectCards {
        cards: vec![first, second],
    })
    .expect("pay the exile cost");
    pass_to_choice(&mut r);
    assert!(
        matches!(
            r.state().waiting_for,
            WaitingFor::ChooseFromZoneChoice { .. }
        ),
        "reach: Zemo offers the copies"
    );
    let result = r
        .act(GameAction::SelectCards {
            cards: vec![first, second],
        })
        .expect("CR 707.12: both copies are cast");
    let copies = r.state().objects.values().filter(|o| o.is_copy).count();
    assert_eq!(copies, 2, "both selected copies are cast");
    assert!(
        result.events.iter().any(|event| matches!(
            event,
            GameEvent::EffectResolved {
                kind: EffectKind::CastCopyOfCard,
                ..
            }
        )),
        "the copy batch completes"
    );
    r.advance_until_stack_empty();
    assert!(
        r.state().stack.is_empty(),
        "the batch and the copies finish"
    );
}

#[test]
fn copy_with_no_spell_ability_is_cast_and_the_batch_continues() {
    zemo_batch(true);
}

#[test]
fn copy_batch_of_instants_control() {
    zemo_batch(false);
}

/// CR 601.2c + CR 115.1 + CR 707.12 (three players): a copy announcement runs
/// the announcing-opponent election of the casting authority. Volcanic
/// Offering's rulings: "you choose the opponents for Volcanic Offering as you
/// cast the spell", the same or different opponents for each effect. The
/// caster elects P2 for the land group and P1 for the creature group; the
/// slots are then announced P0/P2/P0/P1, and the copy stays P0's. `copy`:
/// through Mizzix's Mastery, else the ordinary cast (control).
fn restore(state: &engine::types::game_state::GameState) -> GameRunner {
    use engine::types::game_state::{PersistedGameState, PersistedRestoreFinalization};
    let wire = serde_json::to_value(PersistedGameState::capture(state.clone())).unwrap();
    let restored = serde_json::from_value::<PersistedGameState>(wire)
        .expect("decodes")
        .prepare_for_restore(PersistedRestoreFinalization::DeferUntilRehydrated)
        .expect("admissible")
        .finalize_after_rehydration(|_| Ok(()))
        .expect("publishable");
    GameRunner::from_state(restored)
}

fn volcanic_offering_three_players(copy: bool) {
    let p2 = PlayerId(2);
    let mut s = GameScenario::new_n_player(3, 7);
    s.at_phase(Phase::PreCombatMain);
    let land_p1 = s.add_land_from_oracle(P1, "P1 Nonbasic", "").id();
    let land_p2 = s.add_land_from_oracle(p2, "P2 Nonbasic", "").id();
    let creature_p1 = s.add_creature(P1, "P1 Creature", 3, 12).id();
    let creature_p2 = s.add_creature(p2, "P2 Creature", 3, 12).id();
    let (mut r, mastery, entry) = if copy {
        let offering = s
            .add_spell_to_graveyard(P0, "Volcanic Offering", true)
            .from_oracle_text(VOLCANIC_OFFERING)
            .id();
        let mastery = s
            .add_spell_to_hand(P0, "Mizzix's Mastery", false)
            .from_oracle_text_with_keywords(&["Overload"], MIZZIXS_MASTERY)
            .with_mana_cost(ManaCost::zero())
            .id();
        let mut r = s.build();
        r.cast(mastery).target_object(offering).commit();
        pass_to_choice(&mut r);
        r.act(GameAction::SelectCards {
            cards: vec![offering],
        })
        .expect("cast the copy");
        let WaitingFor::CopyRetarget { copy_id, .. } = r.state().waiting_for else {
            panic!("expected the copy walk, got {:?}", r.state().waiting_for);
        };
        (r, Some(mastery), copy_id)
    } else {
        let offering = s
            .add_spell_to_hand_from_oracle(P0, "Volcanic Offering", true, VOLCANIC_OFFERING)
            .with_mana_cost(ManaCost::zero())
            .id();
        let mut r = s.build();
        begin_cast(&mut r, offering);
        (r, None, offering)
    };
    for (index, elected) in [(1, p2), (2, P1)] {
        let election = match &r.state().waiting_for {
            WaitingFor::ChooseAnnouncingOpponent {
                player,
                candidates,
                choice_index,
                choice_count,
                ..
            } => (*player, candidates.clone(), *choice_index, *choice_count),
            WaitingFor::CopyRetarget {
                player,
                announcer_election: Some(election),
                target_slots,
                ..
            } => {
                assert!(target_slots.is_empty(), "no slot is answered mid-election");
                (
                    *player,
                    election.candidates.clone(),
                    election.choice_index,
                    election.choice_count,
                )
            }
            other => panic!("expected election {index}, got {other:?}"),
        };
        assert_eq!(election.0, P0, "the caster elects");
        assert!(election.1.contains(&P1) && election.1.contains(&p2));
        assert_eq!((election.2, election.3), (index, 2));
        if copy && index == 1 {
            assert!(
                GameRunner::from_state(r.state().clone())
                    .act(GameAction::ChooseTarget {
                        target: Some(TargetRef::Object(land_p1)),
                    })
                    .is_err(),
                "no target is announced before the election"
            );
        }
        if copy {
            // A save mid-election restores to the same election.
            let restored = restore(r.state());
            assert_eq!(
                restored.state().waiting_for,
                r.state().waiting_for,
                "election {index} survives save and restore"
            );
            r = restored;
        }
        r.act(GameAction::ChooseAnnouncingOpponent { opponent: elected })
            .expect("the caster elects an announcing opponent");
    }
    let mut askers: Vec<PlayerId> = Vec::new();
    for target in [land_p1, land_p2, creature_p1, creature_p2] {
        let asker = match r.state().waiting_for.clone() {
            WaitingFor::CopyRetarget {
                player, controller, ..
            } => {
                assert_eq!(controller.unwrap_or(player), P0, "the copy stays P0's");
                if player != P0 {
                    // A save while an opponent answers keeps who answers and
                    // whose copy it is.
                    let restored = restore(r.state());
                    assert_eq!(restored.state().waiting_for, r.state().waiting_for);
                    assert!(matches!(
                        restored.state().waiting_for,
                        WaitingFor::CopyRetarget { player, controller: Some(c), .. }
                            if player != P0 && c == P0
                    ));
                    r = restored;
                }
                player
            }
            WaitingFor::TargetSelection { player, .. } => player,
            other => panic!("expected an announcement prompt, got {other:?}"),
        };
        askers.push(asker);
        r.act(GameAction::ChooseTarget {
            target: Some(TargetRef::Object(target)),
        })
        .expect("the asked player announces");
    }
    assert_eq!(askers, vec![P0, p2, P0, P1]);
    resolve_entry(&mut r, entry);
    assert!([land_p1, land_p2]
        .iter()
        .all(|l| r.state().objects[l].zone == Zone::Graveyard));
    assert_eq!(
        [creature_p1, creature_p2].map(|c| r.state().objects[&c].damage_marked),
        [7, 7]
    );
    if let Some(mastery) = mastery {
        resolve_entry(&mut r, mastery);
        assert_eq!(r.state().objects[&mastery].zone, Zone::Exile);
    }
}

#[test]
fn copy_announcement_elects_announcing_opponents_per_effect() {
    volcanic_offering_three_players(true);
}

#[test]
fn ordinary_volcanic_offering_three_player_election_control() {
    volcanic_offering_three_players(false);
}
