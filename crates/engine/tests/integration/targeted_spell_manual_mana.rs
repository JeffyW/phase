//! phase-rs/phase#9812: a targeted spell payable only with the help of a mana
//! ability the Auto payer can't activate (Treasonous Ogre's "Pay 3 life: Add
//! {R}") must be offered exactly like its untargeted twin. Once targets are
//! chosen, the reducer carries such an Auto cast into the mana-payment step,
//! so the offer seam judges it by that same manual-route authority.
//!
//! Oracle text verbatim from Scryfall `cards/named`.
//!
//! CR 605.1a: "Pay 3 life: Add {R}" is a mana ability.
//! CR 601.2g: mana abilities are activated during casting, before costs are paid.
//! CR 119.4: life can be paid while the life total is at least the payment.

use engine::ai_support::{candidate_actions, legal_actions};
use engine::game::scenario::{GameRunner, GameScenario, P0, P1};
use engine::types::ability::{
    AbilityCost, AbilityDefinition, AbilityKind, ControllerRef, Effect, FilterProp,
    ManaContribution, ManaProduction, TargetFilter, TargetRef, TypedFilter,
};
use engine::types::actions::GameAction;
use engine::types::game_state::{CastPaymentMode, GameState, WaitingFor};
use engine::types::identifiers::ObjectId;
use engine::types::keywords::{FlashbackCost, Keyword};
use engine::types::mana::{ManaColor, ManaCost, ManaCostShard, ManaType, ManaUnit};
use engine::types::phase::Phase;
use engine::types::zones::Zone;

const NYX_LOTUS: &str = "Nyx Lotus enters tapped.\n{T}: Choose a color. Add an amount of mana of that color equal to your devotion to that color. (Your devotion to a color is the number of mana symbols of that color in the mana costs of permanents you control.)";
const TREASONOUS_OGRE: &str = "Dethrone (Whenever this creature attacks the player with the most life or tied for most life, put a +1/+1 counter on it.)\nPay 3 life: Add {R}.";
const UNSUBSTANTIATE: &str = "Return target spell or creature to its owner's hand.";
const WORDS_OF_WISDOM: &str = "You draw two cards, then each other player draws a card.";
const INSPIRATION: &str = "Target player draws two cards.";

fn cost(shards: Vec<ManaCostShard>, generic: u32) -> ManaCost {
    ManaCost::Cost { shards, generic }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum BlueSource {
    /// Nyx Lotus at blue devotion 1 (Merfolk of the Pearl Trident, {U}).
    NyxLotus,
    /// A basic Island (a plain tap source for {U}).
    Island,
    /// {U}{C} already floating (control: no manual mana needed).
    Floating,
}

struct Board {
    runner: GameRunner,
    blue_source: Option<ObjectId>,
    ogre: ObjectId,
    bear: ObjectId,
    unsubstantiate: ObjectId,
    words: ObjectId,
    inspiration: ObjectId,
}

/// P0, own main phase, 20 life, no floating mana unless `Floating`.
/// P0: Treasonous Ogre, Merfolk of the Pearl Trident ({U}), plus the blue source.
/// P1: a Grizzly Bears (a legal target for Unsubstantiate).
fn board(blue: BlueSource) -> Board {
    let mut sc = GameScenario::new();
    sc.at_phase(Phase::PreCombatMain);
    let blue_source = match blue {
        BlueSource::NyxLotus => {
            let mut b = sc.add_artifact_from_oracle(P0, "Nyx Lotus", NYX_LOTUS);
            b.as_legendary().with_mana_cost(cost(vec![], 4));
            Some(b.id())
        }
        BlueSource::Island => Some(sc.add_basic_land(P0, ManaColor::Blue)),
        BlueSource::Floating => {
            sc.with_mana_pool(
                P0,
                vec![
                    ManaUnit::new(ManaType::Blue, ObjectId(0), false, vec![]),
                    ManaUnit::new(ManaType::Colorless, ObjectId(0), false, vec![]),
                ],
            );
            None
        }
    };
    sc.add_creature(P0, "Merfolk of the Pearl Trident", 1, 1)
        .with_mana_cost(cost(vec![ManaCostShard::Blue], 0));
    let ogre = sc
        .add_creature(P0, "Treasonous Ogre", 2, 3)
        .from_oracle_text_with_keywords(&["Dethrone"], TREASONOUS_OGRE)
        .with_mana_cost(cost(vec![ManaCostShard::Red], 3))
        .id();
    let bear = sc.add_creature(P1, "Grizzly Bears", 2, 2).id();
    sc.with_library_top(P0, &["Card A", "Card B", "Card C"]);
    sc.with_library_top(P1, &["Card D", "Card E", "Card F"]);
    let unsubstantiate = sc
        .add_spell_to_hand_from_oracle(P0, "Unsubstantiate", true, UNSUBSTANTIATE)
        .with_mana_cost(cost(vec![ManaCostShard::Blue], 1))
        .id();
    let words = sc
        .add_spell_to_hand_from_oracle(P0, "Words of Wisdom", true, WORDS_OF_WISDOM)
        .with_mana_cost(cost(vec![ManaCostShard::Blue], 1))
        .id();
    let inspiration = sc
        .add_spell_to_hand_from_oracle(P0, "Inspiration", true, INSPIRATION)
        .with_mana_cost(cost(vec![ManaCostShard::Blue], 3))
        .id();
    Board {
        runner: sc.build(),
        blue_source,
        ogre,
        bear,
        unsubstantiate,
        words,
        inspiration,
    }
}

fn offered(runner: &GameRunner, spell: ObjectId) -> bool {
    legal_actions(runner.state())
        .iter()
        .any(|a| matches!(a, GameAction::CastSpell { object_id, .. } if *object_id == spell))
}

/// Reach guards shared by every leg: the Ogre's mana ability really activates
/// from 20 life (on a throwaway copy of the state), and nothing is floating
/// unless the board says so.
fn reach(b: &Board, blue: BlueSource) {
    let s = b.runner.state();
    assert_eq!(s.players[0].life, 20);
    assert_eq!(
        s.players[0].mana_pool.is_empty(),
        blue != BlueSource::Floating
    );
    let mut probe = GameRunner::from_state(s.clone());
    probe
        .act(GameAction::ActivateAbility {
            source_id: b.ogre,
            ability_index: 0,
        })
        .expect("reach: Treasonous Ogre's 'Pay 3 life: Add {R}' activates");
    assert_eq!(probe.state().players[0].life, 17, "reach: 3 life paid");
    assert_eq!(
        probe.state().players[0]
            .mana_pool
            .count_color(ManaType::Red),
        1,
        "reach: {{R}} added"
    );
}

/// The issue board: Nyx Lotus (U at devotion 1) + the Ogre's manual R pays
/// {1}{U}. The untargeted twin is offered; the targeted one must be too.
#[test]
fn targeted_spell_payable_with_ogre_mana_is_offered_nyx_lotus() {
    let b = board(BlueSource::NyxLotus);
    reach(&b, BlueSource::NyxLotus);
    assert_eq!(
        engine::game::devotion::count_devotion(b.runner.state(), P0, &[ManaColor::Blue]),
        1,
        "reach: blue devotion 1, so the Lotus adds exactly U"
    );
    let words = offered(&b.runner, b.words);
    let unsub = offered(&b.runner, b.unsubstantiate);
    let insp = offered(&b.runner, b.inspiration);
    eprintln!("[NyxLotus] Words of Wisdom (untargeted {{1}}{{U}}) offered: {words}");
    eprintln!("[NyxLotus] Unsubstantiate (targeted {{1}}{{U}}) offered: {unsub}");
    eprintln!("[NyxLotus] Inspiration (targeted {{3}}{{U}}) offered: {insp}");
    assert!(
        words,
        "reach: the untargeted {{1}}{{U}} twin is offered (Lotus U + Ogre R)"
    );
    assert!(
        unsub,
        "CR 601.2g + CR 605.1a: the targeted {{1}}{{U}} on the same board is payable the same way \
         and must be offered too"
    );
}

/// Control: a basic Island instead of Nyx Lotus. Island U + Ogre R pays {1}{U}.
/// Tells whether the gap needs the Lotus's chosen-colour profile.
#[test]
fn targeted_spell_payable_with_ogre_mana_is_offered_island() {
    let b = board(BlueSource::Island);
    reach(&b, BlueSource::Island);
    let words = offered(&b.runner, b.words);
    let unsub = offered(&b.runner, b.unsubstantiate);
    eprintln!("[Island] Words of Wisdom (untargeted {{1}}{{U}}) offered: {words}");
    eprintln!("[Island] Unsubstantiate (targeted {{1}}{{U}}) offered: {unsub}");
    assert!(
        words,
        "reach: Island U + Ogre R pays the untargeted {{1}}{{U}}"
    );
    assert!(
        unsub,
        "CR 601.2g + CR 605.1a: Island U + Ogre R pays the targeted {{1}}{{U}}"
    );
}

/// Control: {U}{C} floating, so no manual mana is needed. The targeted spell is
/// offered: target legality isn't what hides it.
#[test]
fn targeted_spell_without_manual_mana_is_offered() {
    let b = board(BlueSource::Floating);
    reach(&b, BlueSource::Floating);
    assert!(offered(&b.runner, b.words));
    assert!(offered(&b.runner, b.unsubstantiate));
}

/// The spell the gate hides is castable: on the Island board, announcing
/// Unsubstantiate anyway and paying Island {U} + the Ogre's {R} by hand casts it,
/// and it resolves (Bears returned to its owner's hand).
#[test]
fn hidden_targeted_spell_is_castable_when_announced() {
    let mut b = board(BlueSource::Island);
    reach(&b, BlueSource::Island);
    let island = b.blue_source.expect("the Island");
    let card_id = b.runner.state().objects[&b.unsubstantiate].card_id;
    b.runner
        .act(GameAction::CastSpell {
            object_id: b.unsubstantiate,
            card_id,
            targets: vec![],
            payment_mode: CastPaymentMode::Manual,
        })
        .expect("Unsubstantiate is announced");
    let mut prompts = Vec::new();
    for _ in 0..16 {
        let wf = b.runner.state().waiting_for.clone();
        prompts.push(
            format!("{wf:?}")
                .split([' ', '{', '('])
                .next()
                .unwrap_or_default()
                .to_string(),
        );
        let action = match wf {
            WaitingFor::TargetSelection { .. } => GameAction::SelectTargets {
                targets: vec![TargetRef::Object(b.bear)],
            },
            WaitingFor::ManaPayment { .. } => {
                if b.runner.state().players[0].life == 20 {
                    GameAction::ActivateAbility {
                        source_id: b.ogre,
                        ability_index: 0,
                    }
                } else if !b.runner.state().objects[&island].tapped {
                    GameAction::ActivateAbility {
                        source_id: island,
                        ability_index: 0,
                    }
                } else {
                    GameAction::PassPriority
                }
            }
            WaitingFor::Priority { .. } if !b.runner.state().stack.is_empty() => {
                GameAction::PassPriority
            }
            WaitingFor::Priority { .. } => break,
            other => panic!("unexpected prompt {other:?}"),
        };
        b.runner.act(action).expect("prompt accepts");
    }
    eprintln!("[Island, announced anyway] prompts: {prompts:?}");
    assert_eq!(
        b.runner.state().players[0].life,
        17,
        "the Ogre paid 3 life for R"
    );
    assert!(
        b.runner.state().objects[&island].tapped,
        "the Island paid U"
    );
    assert_eq!(
        b.runner.state().objects[&b.bear].zone,
        Zone::Hand,
        "Unsubstantiate resolved"
    );
}

fn auto_cast(runner: &GameRunner, spell: ObjectId) -> GameAction {
    GameAction::CastSpell {
        object_id: spell,
        card_id: runner.state().objects[&spell].card_id,
        targets: vec![],
        payment_mode: CastPaymentMode::Auto,
    }
}

/// CR 601.2g + CR 605.3a: the offered Auto action itself completes. After the
/// target is chosen the reducer stops at the mana-payment step (the Auto payer
/// can't pay life), the Ogre's mana ability pays {R}, and passing finishes the
/// payment with the Island. This is what makes the offer honest.
#[test]
fn offered_auto_cast_completes_through_mana_payment() {
    let mut b = board(BlueSource::Island);
    reach(&b, BlueSource::Island);
    let island = b.blue_source.expect("the Island");
    let action = auto_cast(&b.runner, b.unsubstantiate);
    assert!(
        legal_actions(b.runner.state()).contains(&action),
        "the exact Auto action is offered"
    );
    b.runner.act(action).expect("the offered action applies");
    assert!(matches!(
        b.runner.state().waiting_for,
        WaitingFor::TargetSelection { .. }
    ));
    b.runner
        .act(GameAction::SelectTargets {
            targets: vec![TargetRef::Object(b.bear)],
        })
        .expect("the target is accepted");
    assert!(
        matches!(b.runner.state().waiting_for, WaitingFor::ManaPayment { .. }),
        "the Auto payer can't pay life, so the cast waits at the mana-payment step: {:?}",
        b.runner.state().waiting_for
    );
    b.runner
        .act(GameAction::ActivateAbility {
            source_id: b.ogre,
            ability_index: 0,
        })
        .expect("the Ogre's mana ability activates during payment");
    b.runner
        .act(GameAction::PassPriority)
        .expect("passing finishes the payment");
    b.runner.advance_until_stack_empty();
    assert_eq!(b.runner.state().players[0].life, 17, "the Ogre paid 3 life");
    assert!(
        b.runner.state().objects[&island].tapped,
        "the Island paid {{U}}"
    );
    assert_eq!(
        b.runner.state().objects[&b.bear].zone,
        Zone::Hand,
        "Unsubstantiate resolved"
    );
}

/// "Exile a card from your hand: Add {B}." (a choice-bearing, non-tap mana
/// ability; Auto can't activate it).
fn exile_from_hand_mana() -> AbilityDefinition {
    AbilityDefinition::new(
        AbilityKind::Activated,
        Effect::Mana {
            produced: ManaProduction::Fixed {
                colors: vec![ManaColor::Black],
                contribution: ManaContribution::Base,
            },
            restrictions: vec![],
            grants: vec![],
            expiry: None,
            target: None,
        },
    )
    .cost(AbilityCost::Exile {
        count: 1,
        zone: Some(Zone::Hand),
        filter: Some(TargetFilter::Typed(
            TypedFilter::card()
                .controller(ControllerRef::You)
                .properties(vec![FilterProp::InZone { zone: Zone::Hand }]),
        )),
    })
}

struct HandExileBoard {
    runner: GameRunner,
    spell: ObjectId,
    source: ObjectId,
    victim: ObjectId,
}

/// P0: a creature with "Exile a card from your hand: Add {B}", no lands, and
/// one {B} spell in hand ("Destroy target creature." or "Draw a card."). With
/// `fodder`, one more card in hand the source can exile.
fn hand_exile_board(targeted: bool, fodder: bool) -> HandExileBoard {
    let mut sc = GameScenario::new();
    sc.at_phase(Phase::PreCombatMain);
    let source = sc
        .add_creature(P0, "Hand-Exile Mana Source", 0, 3)
        .with_ability_definition(exile_from_hand_mana())
        .id();
    let victim = sc.add_creature(P1, "Victim", 2, 2).id();
    sc.with_library_top(P0, &["Card A", "Card B"]);
    let (name, text) = if targeted {
        ("Targeted Black Spell", "Destroy target creature.")
    } else {
        ("Untargeted Black Spell", "Draw a card.")
    };
    let spell = sc
        .add_spell_to_hand_from_oracle(P0, name, true, text)
        .with_mana_cost(cost(vec![ManaCostShard::Black], 0))
        .id();
    if fodder {
        sc.add_spell_to_hand_from_oracle(P0, "Hand Fodder", true, "Draw a card.")
            .with_mana_cost(cost(vec![ManaCostShard::Red, ManaCostShard::Red], 5));
    }
    HandExileBoard {
        runner: sc.build(),
        spell,
        source,
        victim,
    }
}

/// CR 601.2h: the reducer refuses the payment — at announcement for an
/// untargeted cast, once the target is chosen for a targeted one (after the
/// reach guard that it stopped at target selection, the post-target seam).
fn assert_reducer_refuses_payment(state: &GameState, action: GameAction, victim: Option<ObjectId>) {
    let mut runner = GameRunner::from_state(state.clone());
    let refused = match victim {
        None => runner.act(action),
        Some(victim) => {
            runner.act(action).expect("the targeted cast is announced");
            assert!(
                matches!(
                    runner.state().waiting_for,
                    WaitingFor::TargetSelection { .. }
                ),
                "reach: the targeted cast stops at target selection"
            );
            runner.act(GameAction::SelectTargets {
                targets: vec![TargetRef::Object(victim)],
            })
        }
    };
    assert!(
        refused.is_err(),
        "the spell being cast can't pay for itself: {:?}",
        runner.state().waiting_for
    );
}

fn announce_and_target(runner: &mut GameRunner, action: GameAction, victim: Option<ObjectId>) {
    runner.act(action).expect("the cast is announced");
    if let Some(victim) = victim {
        assert!(
            matches!(
                runner.state().waiting_for,
                WaitingFor::TargetSelection { .. }
            ),
            "reach: the targeted cast stops at target selection, the post-target offer seam"
        );
        runner
            .act(GameAction::SelectTargets {
                targets: vec![TargetRef::Object(victim)],
            })
            .expect("the target is accepted");
    }
    assert!(
        matches!(runner.state().waiting_for, WaitingFor::ManaPayment { .. }),
        "reach: the cast waits at the mana-payment step: {:?}",
        runner.state().waiting_for
    );
}

/// Hostile (CR 601.2a + CR 601.2g): before the cast is announced the spell in
/// hand is fodder for "Exile a card from your hand: Add {B}", so the broad
/// castability gate generates the cast. Once announced, the spell is on the
/// stack: the source has nothing to exile, the cast can't be paid, and it is
/// not offered, targeted or not. (Untargeted, this was offered before the fix,
/// and the source could exile the spell being cast.)
#[test]
fn spell_that_is_its_own_only_exile_fodder_is_not_offered() {
    for targeted in [false, true] {
        let b = hand_exile_board(targeted, false);
        let action = auto_cast(&b.runner, b.spell);
        assert!(
            candidate_actions(b.runner.state())
                .iter()
                .any(|candidate| candidate.action == action),
            "reach (targeted={targeted}): before announcement the spell is its own fodder"
        );
        assert!(
            !legal_actions(b.runner.state()).contains(&action),
            "targeted={targeted}: nothing is left to exile once the spell is on the stack"
        );
        assert_reducer_refuses_payment(b.runner.state(), action, targeted.then_some(b.victim));
    }
}

/// Positive control for the self-fodder board: one other card in hand makes the
/// targeted cast payable through the exile source, so it is offered, and the
/// offered Auto action completes with that card (the only choice).
#[test]
fn spell_with_other_exile_fodder_is_offered_and_completes() {
    let mut b = hand_exile_board(true, true);
    let action = auto_cast(&b.runner, b.spell);
    assert!(legal_actions(b.runner.state()).contains(&action));
    announce_and_target(&mut b.runner, action, Some(b.victim));
    b.runner
        .act(GameAction::ActivateAbility {
            source_id: b.source,
            ability_index: 0,
        })
        .expect("the exile source activates during payment");
    let WaitingFor::PayCost { choices, .. } = b.runner.state().waiting_for.clone() else {
        panic!(
            "the exile cost asks for a card: {:?}",
            b.runner.state().waiting_for
        )
    };
    assert!(
        !choices.contains(&b.spell),
        "CR 601.2a: the spell being cast is not in hand"
    );
    assert_eq!(choices.len(), 1, "only the other card remains in hand");
    b.runner
        .act(GameAction::SelectCards { cards: choices })
        .expect("the other card is exiled");
    b.runner
        .act(GameAction::PassPriority)
        .expect("passing finishes the payment");
    b.runner.advance_until_stack_empty();
    assert_eq!(b.runner.state().objects[&b.victim].zone, Zone::Graveyard);
}

const SKIRGE_FAMILIAR: &str = "Flying\nDiscard a card: Add {B}.";

/// P0: Skirge Familiar (Oracle text verbatim from Scryfall), no lands, and a
/// {B} spell in hand, plus one other card when `other_card`.
fn skirge_board(targeted: bool, other_card: bool) -> HandExileBoard {
    let mut sc = GameScenario::new();
    sc.at_phase(Phase::PreCombatMain);
    let source = sc
        .add_creature(P0, "Skirge Familiar", 3, 2)
        .from_oracle_text_with_keywords(&["Flying"], SKIRGE_FAMILIAR)
        .with_mana_cost(cost(vec![ManaCostShard::Black], 4))
        .id();
    let victim = sc.add_creature(P1, "Victim", 2, 2).id();
    sc.with_library_top(P0, &["Card A", "Card B"]);
    let (name, text) = if targeted {
        ("Targeted Black Spell", "Destroy target creature.")
    } else {
        ("Untargeted Black Spell", "Draw a card.")
    };
    let spell = sc
        .add_spell_to_hand_from_oracle(P0, name, true, text)
        .with_mana_cost(cost(vec![ManaCostShard::Black], 0))
        .id();
    if other_card {
        sc.add_spell_to_hand_from_oracle(P0, "Other Card", true, "Draw a card.")
            .with_mana_cost(cost(vec![ManaCostShard::Red, ManaCostShard::Red], 5));
    }
    HandExileBoard {
        runner: sc.build(),
        spell,
        source,
        victim,
    }
}

/// CR 601.2a + CR 601.2g: Skirge Familiar can't discard the spell being cast
/// to pay for it. With that spell as the only card in hand, the cast isn't
/// offered, targeted or untargeted. (Before the fix the untargeted cast was
/// offered, and discarding the spell being cast paid for it.)
#[test]
fn skirge_familiar_cannot_discard_the_spell_being_cast() {
    for targeted in [false, true] {
        let b = skirge_board(targeted, false);
        let action = auto_cast(&b.runner, b.spell);
        assert!(
            candidate_actions(b.runner.state())
                .iter()
                .any(|candidate| candidate.action == action),
            "reach (targeted={targeted}): before announcement the spell is the Skirge's fodder"
        );
        assert!(
            !legal_actions(b.runner.state()).contains(&action),
            "targeted={targeted}: no card is left to discard once the spell is on the stack"
        );
        assert_reducer_refuses_payment(b.runner.state(), action, targeted.then_some(b.victim));
    }
}

/// Positive control: with one other card in hand, Skirge Familiar pays for the
/// targeted spell by discarding that card, the only choice.
#[test]
fn skirge_familiar_pays_by_discarding_another_card() {
    let mut b = skirge_board(true, true);
    let action = auto_cast(&b.runner, b.spell);
    assert!(legal_actions(b.runner.state()).contains(&action));
    announce_and_target(&mut b.runner, action, Some(b.victim));
    b.runner
        .act(GameAction::ActivateAbility {
            source_id: b.source,
            ability_index: 0,
        })
        .expect("Skirge Familiar activates during payment");
    let WaitingFor::PayCost { choices, .. } = b.runner.state().waiting_for.clone() else {
        panic!(
            "the discard cost asks for a card: {:?}",
            b.runner.state().waiting_for
        )
    };
    assert_eq!(choices.len(), 1, "only the other card can be discarded");
    assert!(!choices.contains(&b.spell));
    b.runner
        .act(GameAction::SelectCards { cards: choices })
        .expect("the other card is discarded");
    b.runner
        .act(GameAction::PassPriority)
        .expect("passing finishes the payment");
    b.runner.advance_until_stack_empty();
    assert_eq!(b.runner.state().objects[&b.victim].zone, Zone::Graveyard);
}

/// "Exile a card from your graveyard: Add {B}."
fn exile_from_graveyard_mana() -> AbilityDefinition {
    AbilityDefinition::new(
        AbilityKind::Activated,
        Effect::Mana {
            produced: ManaProduction::Fixed {
                colors: vec![ManaColor::Black],
                contribution: ManaContribution::Base,
            },
            restrictions: vec![],
            grants: vec![],
            expiry: None,
            target: None,
        },
    )
    .cost(AbilityCost::Exile {
        count: 1,
        zone: Some(Zone::Graveyard),
        filter: Some(TargetFilter::Typed(
            TypedFilter::card()
                .controller(ControllerRef::You)
                .properties(vec![FilterProp::InZone {
                    zone: Zone::Graveyard,
                }]),
        )),
    })
}

/// P0: a creature with "Exile a card from your graveyard: Add {B}", no lands,
/// and a targeted {B} flashback spell in the graveyard, plus one other card
/// there when `other_card`.
fn graveyard_exile_board(other_card: bool) -> HandExileBoard {
    let mut sc = GameScenario::new();
    sc.at_phase(Phase::PreCombatMain);
    let source = sc
        .add_creature(P0, "Graveyard-Exile Mana Source", 0, 3)
        .with_ability_definition(exile_from_graveyard_mana())
        .id();
    let victim = sc.add_creature(P1, "Victim", 2, 2).id();
    let black = cost(vec![ManaCostShard::Black], 0);
    let spell = sc
        .add_spell_to_graveyard(P0, "Flashback Black Spell", true)
        .from_oracle_text("Destroy target creature.")
        .with_mana_cost(black.clone())
        .with_keyword(Keyword::Flashback(FlashbackCost::Mana(black)))
        .id();
    if other_card {
        sc.add_spell_to_graveyard(P0, "Other Graveyard Card", true);
    }
    HandExileBoard {
        runner: sc.build(),
        spell,
        source,
        victim,
    }
}

fn flashback_cast(runner: &GameRunner, spell: ObjectId) -> Option<GameAction> {
    candidate_actions(runner.state())
        .into_iter()
        .map(|candidate| candidate.action)
        .find(|action| {
            matches!(action, GameAction::CastSpell { object_id, .. } if *object_id == spell)
        })
}

/// CR 601.2a + CR 702.34a: a flashback spell cast from the graveyard is on the
/// stack, so it can't be exiled from the graveyard to pay for itself.
#[test]
fn flashback_spell_cannot_exile_itself_from_the_graveyard_to_pay() {
    let b = graveyard_exile_board(false);
    let action = flashback_cast(&b.runner, b.spell)
        .expect("reach: before announcement the spell is its own graveyard fodder");
    assert!(
        !legal_actions(b.runner.state()).contains(&action),
        "nothing is left to exile once the spell is on the stack"
    );
    assert_reducer_refuses_payment(b.runner.state(), action, Some(b.victim));
}

/// Positive control: another graveyard card pays for the flashback cast.
#[test]
fn flashback_spell_pays_by_exiling_another_graveyard_card() {
    let mut b = graveyard_exile_board(true);
    let action = flashback_cast(&b.runner, b.spell).expect("the flashback cast is generated");
    assert!(legal_actions(b.runner.state()).contains(&action));
    announce_and_target(&mut b.runner, action, Some(b.victim));
    b.runner
        .act(GameAction::ActivateAbility {
            source_id: b.source,
            ability_index: 0,
        })
        .expect("the exile source activates during payment");
    let WaitingFor::PayCost { choices, .. } = b.runner.state().waiting_for.clone() else {
        panic!(
            "the exile cost asks for a card: {:?}",
            b.runner.state().waiting_for
        )
    };
    assert_eq!(choices.len(), 1, "only the other card can be exiled");
    assert!(!choices.contains(&b.spell));
    b.runner
        .act(GameAction::SelectCards { cards: choices })
        .expect("the other card is exiled");
    b.runner
        .act(GameAction::PassPriority)
        .expect("passing finishes the payment");
    b.runner.advance_until_stack_empty();
    assert_eq!(b.runner.state().objects[&b.victim].zone, Zone::Graveyard);
    assert_eq!(
        b.runner.state().objects[&b.spell].zone,
        Zone::Exile,
        "CR 702.34a: the flashback spell is exiled as it leaves the stack"
    );
}

const EVIDENCE_SPELL: &str =
    "As an additional cost to cast this spell, collect evidence 3.\nDraw a card.";

/// P0: a sorcery with "collect evidence 3" and flashback {0} in the graveyard
/// (mana value 3), plus, when `fuel`, another mana value 3 card there.
fn evidence_board(fuel: bool) -> (GameRunner, ObjectId, Option<ObjectId>) {
    let mut sc = GameScenario::new();
    sc.at_phase(Phase::PreCombatMain);
    sc.with_library_top(P0, &["Card A", "Card B"]);
    let spell = sc
        .add_spell_to_graveyard(P0, "Evidence Flashback Spell", false)
        .from_oracle_text(EVIDENCE_SPELL)
        .with_mana_cost(ManaCost::generic(3))
        .with_keyword(Keyword::Flashback(FlashbackCost::Mana(ManaCost::zero())))
        .id();
    let fuel = fuel.then(|| {
        sc.add_spell_to_graveyard(P0, "Evidence Fuel", false)
            .with_mana_cost(ManaCost::generic(3))
            .id()
    });
    (sc.build(), spell, fuel)
}

/// CR 601.2a + CR 701.59a: a collect-evidence spell cast from the graveyard is
/// on the stack, so it can't exile itself as its own evidence. With no other
/// card there, the reducer refuses the cast. (Whether such a cast is offered
/// at all is a separate pre-existing gap: the castability gate doesn't check a
/// required collect-evidence cost, even with an empty graveyard. Not asserted.)
#[test]
fn collect_evidence_spell_is_not_its_own_evidence() {
    // Reach guard: with other evidence the same flashback cast is generated
    // and reaches the evidence choice (see the positive control below).
    let (fueled, fueled_spell, _) = evidence_board(true);
    assert!(flashback_cast(&fueled, fueled_spell).is_some());

    let (runner, spell, _) = evidence_board(false);
    let action = GameAction::CastSpell {
        object_id: spell,
        card_id: runner.state().objects[&spell].card_id,
        targets: vec![],
        payment_mode: CastPaymentMode::Auto,
    };
    let mut announced = GameRunner::from_state(runner.state().clone());
    assert!(
        announced.act(action).is_err(),
        "the reducer refuses a cast whose only evidence is itself"
    );
}

/// Positive control: another mana value 3 card is the only evidence offered,
/// and the cast resolves.
#[test]
fn collect_evidence_from_graveyard_uses_other_cards() {
    let (mut runner, spell, fuel) = evidence_board(true);
    let fuel = fuel.expect("fuel card");
    let action = flashback_cast(&runner, spell).expect("the flashback cast is generated");
    assert!(legal_actions(runner.state()).contains(&action));
    runner.act(action).expect("the cast is announced");
    let WaitingFor::CollectEvidenceChoice { cards, .. } = runner.state().waiting_for.clone() else {
        panic!(
            "the cast asks for evidence: {:?}",
            runner.state().waiting_for
        )
    };
    assert_eq!(
        cards,
        vec![fuel],
        "CR 601.2a: the spell being cast isn't evidence"
    );
    runner
        .act(GameAction::SelectCards { cards })
        .expect("the evidence is collected");
    let hand_before = runner.state().players[0].hand.len();
    runner.advance_until_stack_empty();
    assert_eq!(runner.state().players[0].hand.len(), hand_before + 1);
    assert_eq!(runner.state().objects[&fuel].zone, Zone::Exile);
    assert_eq!(runner.state().objects[&spell].zone, Zone::Exile);
}

const VICTIM_OF_NIGHT: &str = "Destroy target non-Vampire, non-Werewolf, non-Zombie creature.";

struct TwinSkirgeBoard {
    runner: GameRunner,
    victim_of_night: ObjectId,
    skirges: [ObjectId; 2],
    bears: ObjectId,
    other_cards: Vec<ObjectId>,
}

/// Probe R1: P0 in its main phase, empty pool, no lands, two Skirge Familiars,
/// Victim of Night ({B}{B}) and `other_cards` other cards in hand; P1 has
/// Grizzly Bears.
fn twin_skirge_board(other_cards: usize) -> TwinSkirgeBoard {
    let mut sc = GameScenario::new();
    sc.at_phase(Phase::PreCombatMain);
    let skirge = |sc: &mut GameScenario| {
        sc.add_creature(P0, "Skirge Familiar", 3, 2)
            .from_oracle_text_with_keywords(&["Flying"], SKIRGE_FAMILIAR)
            .with_mana_cost(cost(vec![ManaCostShard::Black], 4))
            .id()
    };
    let skirges = [skirge(&mut sc), skirge(&mut sc)];
    let bears = sc.add_creature(P1, "Grizzly Bears", 2, 2).id();
    let victim_of_night = sc
        .add_spell_to_hand_from_oracle(P0, "Victim of Night", true, VICTIM_OF_NIGHT)
        .with_mana_cost(cost(vec![ManaCostShard::Black, ManaCostShard::Black], 0))
        .id();
    let other_cards = (0..other_cards)
        .map(|i| {
            sc.add_spell_to_hand_from_oracle(P0, &format!("Other Card {i}"), true, "Draw a card.")
                .with_mana_cost(cost(vec![ManaCostShard::Red, ManaCostShard::Red], 5))
                .id()
        })
        .collect();
    TwinSkirgeBoard {
        runner: sc.build(),
        victim_of_night,
        skirges,
        bears,
        other_cards,
    }
}

/// Probe R1 (CR 601.2a + CR 601.2h + CR 118.3): two Skirge Familiars can't
/// both discard the one other card in hand, so Victim of Night's {B}{B} can't
/// be paid and the cast is not offered (the payment is unproven). Reach
/// guards: the broad gate generates it, its locked cost is {B}{B}, and, cast
/// anyway, the reducer parks it at the mana-payment step as it did before this
/// change, where one Familiar's discard prompt holds only the other card and
/// after that discard the second Familiar can't be activated.
#[test]
fn two_discard_sources_cannot_share_one_card() {
    let b = twin_skirge_board(1);
    let action = auto_cast(&b.runner, b.victim_of_night);
    assert!(
        candidate_actions(b.runner.state())
            .iter()
            .any(|candidate| candidate.action == action),
        "reach: the broad gate generates the cast"
    );
    assert!(
        !legal_actions(b.runner.state()).contains(&action),
        "one card can't pay two Familiars' discard costs"
    );

    let mut announced = GameRunner::from_state(b.runner.state().clone());
    announced.act(action).expect("the cast is announced");
    let WaitingFor::TargetSelection { pending_cast, .. } = &announced.state().waiting_for else {
        panic!("reach: target selection")
    };
    assert_eq!(
        pending_cast.cost,
        cost(vec![ManaCostShard::Black, ManaCostShard::Black], 0),
        "reach: the locked cost is {{B}}{{B}}"
    );
    announced
        .act(GameAction::SelectTargets {
            targets: vec![TargetRef::Object(b.bears)],
        })
        .expect("the target is accepted");
    {
        assert!(matches!(
            announced.state().waiting_for,
            WaitingFor::ManaPayment { .. }
        ));
        announced
            .act(GameAction::ActivateAbility {
                source_id: b.skirges[0],
                ability_index: 0,
            })
            .expect("the first Familiar activates");
        let WaitingFor::PayCost { choices, .. } = announced.state().waiting_for.clone() else {
            panic!("reach: the discard prompt")
        };
        assert_eq!(
            choices, b.other_cards,
            "reach: only the other card can be discarded"
        );
        announced
            .act(GameAction::SelectCards { cards: choices })
            .expect("the other card is discarded");
        assert!(
            announced
                .act(GameAction::ActivateAbility {
                    source_id: b.skirges[1],
                    ability_index: 0,
                })
                .is_err(),
            "the second Familiar has nothing left to discard"
        );
    }
}

/// Positive sibling: with two other cards, each Familiar discards one and
/// Victim of Night resolves.
#[test]
fn two_discard_sources_with_two_cards_pay_and_resolve() {
    let mut b = twin_skirge_board(2);
    let action = auto_cast(&b.runner, b.victim_of_night);
    assert!(legal_actions(b.runner.state()).contains(&action));
    announce_and_target(&mut b.runner, action, Some(b.bears));
    for skirge in b.skirges {
        b.runner
            .act(GameAction::ActivateAbility {
                source_id: skirge,
                ability_index: 0,
            })
            .expect("a Familiar activates");
        let WaitingFor::PayCost { choices, .. } = b.runner.state().waiting_for.clone() else {
            panic!("the discard prompt: {:?}", b.runner.state().waiting_for)
        };
        assert!(!choices.contains(&b.victim_of_night));
        b.runner
            .act(GameAction::SelectCards {
                cards: vec![choices[0]],
            })
            .expect("one other card is discarded");
    }
    b.runner
        .act(GameAction::PassPriority)
        .expect("passing finishes the payment");
    b.runner.advance_until_stack_empty();
    assert_eq!(b.runner.state().objects[&b.bears].zone, Zone::Graveyard);
}

/// CR 119.4 + CR 118.3: two Treasonous Ogres share one life total. At 5 life
/// the second "Pay 3 life" can't be paid, so a targeted {R}{R} spell's payment
/// is unproven and the cast isn't offered; at 6 life both payments fit and it
/// is (targeted and untargeted).
#[test]
fn two_life_sources_share_one_life_total() {
    for (life, targeted, offered) in [(5, true, false), (6, true, true), (6, false, true)] {
        {
            let mut sc = GameScenario::new();
            sc.at_phase(Phase::PreCombatMain);
            sc.with_life(P0, life);
            for _ in 0..2 {
                sc.add_creature(P0, "Treasonous Ogre", 2, 3)
                    .from_oracle_text_with_keywords(&["Dethrone"], TREASONOUS_OGRE)
                    .with_mana_cost(cost(vec![ManaCostShard::Red], 3));
            }
            sc.add_creature(P1, "Grizzly Bears", 2, 2);
            sc.with_library_top(P0, &["Card A", "Card B"]);
            let text = if targeted {
                "Destroy target creature."
            } else {
                "Draw a card."
            };
            let spell = sc
                .add_spell_to_hand_from_oracle(P0, "Red Red Spell", true, text)
                .with_mana_cost(cost(vec![ManaCostShard::Red, ManaCostShard::Red], 0))
                .id();
            let runner = sc.build();
            let action = auto_cast(&runner, spell);
            assert_eq!(
                legal_actions(runner.state()).contains(&action),
                offered,
                "life {life}, targeted={targeted}"
            );
        }
    }
}

const CITANUL_STALWART: &str =
    "{T}, Tap an untapped artifact or creature you control: Add one mana of any color.";

/// Probe R3-A (CR 118.3 + CR 601.2h): two Citanul Stalwarts and `helpers`
/// extra Grizzly Bears for P0, Victim of Night ({B}{B}) in hand, P1's Bears.
fn twin_stalwart_board(helpers: usize) -> (GameRunner, ObjectId, [ObjectId; 2], ObjectId) {
    let mut sc = GameScenario::new();
    sc.at_phase(Phase::PreCombatMain);
    let stalwart = |sc: &mut GameScenario| {
        sc.add_creature(P0, "Citanul Stalwart", 1, 1)
            .from_oracle_text(CITANUL_STALWART)
            .with_mana_cost(cost(vec![ManaCostShard::Green], 0))
            .id()
    };
    let stalwarts = [stalwart(&mut sc), stalwart(&mut sc)];
    for _ in 0..helpers {
        sc.add_creature(P0, "Grizzly Bears", 2, 2);
    }
    let target = sc.add_creature(P1, "Grizzly Bears", 2, 2).id();
    let victim_of_night = sc
        .add_spell_to_hand_from_oracle(P0, "Victim of Night", true, VICTIM_OF_NIGHT)
        .with_mana_cost(cost(vec![ManaCostShard::Black, ManaCostShard::Black], 0))
        .id();
    (sc.build(), victim_of_night, stalwarts, target)
}

/// Answer the Stalwart's helper prompt with a helper that is not a Stalwart and the color
/// prompt with black, until the mana-payment step returns.
fn activate_stalwart(
    runner: &mut GameRunner,
    stalwart: ObjectId,
    stalwarts: &[ObjectId],
) -> Result<(), String> {
    runner
        .act(GameAction::ActivateAbility {
            source_id: stalwart,
            ability_index: 0,
        })
        .map_err(|e| format!("{e:?}"))?;
    for _ in 0..4 {
        match runner.state().waiting_for.clone() {
            WaitingFor::PayCost { choices, .. } => {
                let helper = choices
                    .iter()
                    .copied()
                    .find(|id| !stalwarts.contains(id))
                    .ok_or("no helper other than a Stalwart")?;
                runner
                    .act(GameAction::SelectCards {
                        cards: vec![helper],
                    })
                    .map_err(|e| format!("{e:?}"))?;
            }
            WaitingFor::ChooseManaColor { .. } => {
                runner
                    .act(GameAction::ChooseManaColor {
                        choice: engine::types::game_state::ManaChoice::SingleColor(ManaType::Black),
                        count: 1,
                    })
                    .map_err(|e| format!("{e:?}"))?;
            }
            _ => return Ok(()),
        }
    }
    Ok(())
}

/// Probe R3-A: with one helper Bears, the two Stalwarts can't both tap an
/// untapped creature (each Stalwart taps itself; the other Stalwart can't be
/// tapped for its partner and still activate), so only one {B} is reachable and
/// the cast is not offered. Reach guards: the broad gate generates it, its cost
/// is {B}{B}, and cast anyway the first Stalwart activates while the second has
/// no helper left.
#[test]
fn two_tap_helper_sources_cannot_share_one_helper() {
    let (runner, victim_of_night, stalwarts, target) = twin_stalwart_board(1);
    let action = auto_cast(&runner, victim_of_night);
    assert!(
        candidate_actions(runner.state())
            .iter()
            .any(|candidate| candidate.action == action),
        "reach: the broad gate generates the cast"
    );
    assert!(
        !legal_actions(runner.state()).contains(&action),
        "one helper can't be tapped for both Stalwarts"
    );
    let mut announced = GameRunner::from_state(runner.state().clone());
    announced.act(action).expect("the cast is announced");
    let WaitingFor::TargetSelection { pending_cast, .. } = &announced.state().waiting_for else {
        panic!("reach: target selection")
    };
    assert_eq!(
        pending_cast.cost,
        cost(vec![ManaCostShard::Black, ManaCostShard::Black], 0)
    );
    announced
        .act(GameAction::SelectTargets {
            targets: vec![TargetRef::Object(target)],
        })
        .expect("the target is accepted");
    assert!(matches!(
        announced.state().waiting_for,
        WaitingFor::ManaPayment { .. }
    ));
    activate_stalwart(&mut announced, stalwarts[0], &stalwarts)
        .expect("the first Stalwart activates");
    assert_eq!(
        announced.state().players[0]
            .mana_pool
            .count_color(ManaType::Black),
        1,
        "reach: the first Stalwart added {{B}}"
    );
    assert!(
        activate_stalwart(&mut announced, stalwarts[1], &stalwarts).is_err(),
        "the second Stalwart has no untapped helper left"
    );
}

/// Positive sibling: a second helper Bears lets both Stalwarts activate.
#[test]
fn two_tap_helper_sources_with_two_helpers_pay_and_resolve() {
    let (mut runner, victim_of_night, stalwarts, target) = twin_stalwart_board(2);
    let action = auto_cast(&runner, victim_of_night);
    assert!(legal_actions(runner.state()).contains(&action));
    announce_and_target(&mut runner, action, Some(target));
    for stalwart in stalwarts {
        activate_stalwart(&mut runner, stalwart, &stalwarts).expect("a Stalwart activates");
    }
    runner
        .act(GameAction::PassPriority)
        .expect("passing finishes the payment");
    runner.advance_until_stack_empty();
    assert_eq!(runner.state().objects[&target].zone, Zone::Graveyard);
}

/// Multi-ability infrastructure fixture: Skirge Familiar's printed ability plus
/// a synthetic "{T}: Add {C}." sibling.
const SKIRGE_WITH_FREE_SIBLING: &str = "Flying\nDiscard a card: Add {B}.\n{T}: Add {C}.";

fn twin_skirge_with_free_sibling_board(other_cards: usize) -> (GameRunner, ObjectId, ObjectId) {
    let mut sc = GameScenario::new();
    sc.at_phase(Phase::PreCombatMain);
    for _ in 0..2 {
        sc.add_creature(P0, "Skirge Familiar", 3, 2)
            .from_oracle_text_with_keywords(&["Flying"], SKIRGE_WITH_FREE_SIBLING)
            .with_mana_cost(cost(vec![ManaCostShard::Black], 4));
    }
    let bears = sc.add_creature(P1, "Grizzly Bears", 2, 2).id();
    let victim_of_night = sc
        .add_spell_to_hand_from_oracle(P0, "Victim of Night", true, VICTIM_OF_NIGHT)
        .with_mana_cost(cost(vec![ManaCostShard::Black, ManaCostShard::Black], 0))
        .id();
    for i in 0..other_cards {
        sc.add_spell_to_hand_from_oracle(P0, &format!("Other Card {i}"), true, "Draw a card.")
            .with_mana_cost(cost(vec![ManaCostShard::Red, ManaCostShard::Red], 5));
    }
    (sc.build(), victim_of_night, bears)
}

/// Probe R3-B: a free "{T}: Add {C}" sibling doesn't erase the discard
/// ability's demand. With one other card, {B}{B} still needs two discards, so
/// the cast is not offered; with two other cards it is.
#[test]
fn free_sibling_ability_does_not_erase_discard_demand() {
    let (runner, victim_of_night, _) = twin_skirge_with_free_sibling_board(1);
    let action = auto_cast(&runner, victim_of_night);
    assert!(
        candidate_actions(runner.state())
            .iter()
            .any(|candidate| candidate.action == action),
        "reach: the broad gate generates the cast"
    );
    assert!(!legal_actions(runner.state()).contains(&action));

    let (payable, payable_spell, _) = twin_skirge_with_free_sibling_board(2);
    assert!(legal_actions(payable.state()).contains(&auto_cast(&payable, payable_spell)));
}

/// Probe R4 (CR 117.1d + CR 601.2g-h): however many discard sources are on the
/// battlefield, an untargeted {1}{U} payable with the Island and one discard is
/// offered, and the Auto cast completes through the mana-payment step.
/// Controls: ten Familiars, and eleven with {U}{C} already floating.
#[test]
fn many_discard_sources_do_not_hide_a_cast_one_of_them_pays() {
    for (familiars, floating) in [(11, false), (10, false), (11, true)] {
        let mut sc = GameScenario::new();
        sc.at_phase(Phase::PreCombatMain);
        let island = sc.add_basic_land(P0, ManaColor::Blue);
        let skirges: Vec<ObjectId> = (0..familiars)
            .map(|_| {
                sc.add_creature(P0, "Skirge Familiar", 3, 2)
                    .from_oracle_text_with_keywords(&["Flying"], SKIRGE_FAMILIAR)
                    .with_mana_cost(cost(vec![ManaCostShard::Black], 4))
                    .id()
            })
            .collect();
        sc.with_library_top(P0, &["Card A", "Card B", "Card C"]);
        sc.with_library_top(P1, &["Card D", "Card E"]);
        if floating {
            sc.with_mana_pool(
                P0,
                vec![
                    ManaUnit::new(ManaType::Blue, ObjectId(0), false, vec![]),
                    ManaUnit::new(ManaType::Colorless, ObjectId(0), false, vec![]),
                ],
            );
        }
        let words = sc
            .add_spell_to_hand_from_oracle(P0, "Words of Wisdom", true, WORDS_OF_WISDOM)
            .with_mana_cost(cost(vec![ManaCostShard::Blue], 1))
            .id();
        for i in 0..2 {
            sc.add_spell_to_hand_from_oracle(P0, &format!("Other Card {i}"), true, "Draw a card.")
                .with_mana_cost(cost(vec![ManaCostShard::Red, ManaCostShard::Red], 5));
        }
        let mut runner = sc.build();
        let action = auto_cast(&runner, words);
        assert!(
            legal_actions(runner.state()).contains(&action),
            "{familiars} Familiars, floating={floating}: offered"
        );
        runner.act(action).expect("the cast is announced");
        if !floating {
            assert!(
                matches!(runner.state().waiting_for, WaitingFor::ManaPayment { .. }),
                "{familiars} Familiars: the Auto cast waits at the mana-payment step"
            );
            runner
                .act(GameAction::ActivateAbility {
                    source_id: skirges[0],
                    ability_index: 0,
                })
                .expect("one Familiar activates");
            let WaitingFor::PayCost { choices, .. } = runner.state().waiting_for.clone() else {
                panic!("the discard prompt")
            };
            runner
                .act(GameAction::SelectCards {
                    cards: vec![choices[0]],
                })
                .expect("one card is discarded");
            runner
                .act(GameAction::PassPriority)
                .expect("the Island finishes the payment");
            assert!(runner.state().objects[&island].tapped);
        }
        runner.advance_until_stack_empty();
        assert_eq!(
            runner.state().objects[&words].zone,
            Zone::Graveyard,
            "{familiars} Familiars, floating={floating}: Words of Wisdom resolved"
        );
    }
}

/// Probe R6 board: "Exile a card from your hand: Add {B}" (labelled
/// infrastructure fixture), a targeted {B} spell with "As an additional cost
/// to cast this spell, discard a card at random.", and `other_cards` other
/// cards in hand. With `swamp`, a Swamp instead of the exile source.
fn random_discard_spell_board(other_cards: usize, swamp: bool) -> (GameRunner, ObjectId) {
    let mut sc = GameScenario::new();
    sc.at_phase(Phase::PreCombatMain);
    if swamp {
        sc.add_basic_land(P0, ManaColor::Black);
    } else {
        sc.add_creature(P0, "Hand-Exile Mana Source", 0, 3)
            .with_ability_definition(exile_from_hand_mana());
    }
    sc.add_creature(P1, "Grizzly Bears", 2, 2);
    let spell = sc
        .add_spell_to_hand_from_oracle(
            P0,
            "Random Discard Removal",
            true,
            "As an additional cost to cast this spell, discard a card at random.\nDestroy target creature.",
        )
        .with_mana_cost(cost(vec![ManaCostShard::Black], 0))
        .id();
    for i in 0..other_cards {
        sc.add_spell_to_hand_from_oracle(P0, &format!("Other Card {i}"), true, "Draw a card.")
            .with_mana_cost(cost(vec![ManaCostShard::Red, ManaCostShard::Red], 5));
    }
    (sc.build(), spell)
}

/// Probe R6 (CR 601.2b + CR 601.2h): a cast that still owes another cost (here
/// a random discard from the same hand the exile source draws on) never has a
/// proven manual payment, so it isn't offered with one other card or with two.
/// Reach guards: the spell carries the additional cost, and the broad gate
/// generates the cast. Base control: with a Swamp the Auto payer pays it and
/// the cast is offered.
#[test]
fn outstanding_additional_cost_is_not_a_proven_payment() {
    for other_cards in [1, 2] {
        let (runner, spell) = random_discard_spell_board(other_cards, false);
        assert!(
            runner.state().objects[&spell].additional_cost.is_some(),
            "reach: the spell carries its random-discard additional cost"
        );
        let action = auto_cast(&runner, spell);
        assert!(
            candidate_actions(runner.state())
                .iter()
                .any(|candidate| candidate.action == action),
            "reach ({other_cards} other cards): the broad gate generates the cast"
        );
        assert!(
            !legal_actions(runner.state()).contains(&action),
            "{other_cards} other cards: the outstanding discard keeps the payment unproven"
        );
    }
    let (control, control_spell) = random_discard_spell_board(1, true);
    assert!(legal_actions(control.state()).contains(&auto_cast(&control, control_spell)));
}

const SOLDEVI_ADNATE: &str =
    "{T}, Sacrifice a black or artifact creature: Add an amount of {B} equal to the sacrificed creature's mana value.";

/// Variable-yield infrastructure fixture: Soldevi Adnate's parsed output ("an
/// amount of {B} equal to the ... mana value" of the cost-paid object) behind
/// "{T}, Exile a card from your graveyard" instead of its sacrifice, so the
/// cast is generated with plain Auto payment and the offer consults the
/// payment proof. Two such sources; the graveyard holds a mana value 4 card
/// and `second` (mana value 4 or 0).
fn exile_sized_board(second_mana_value: u32) -> (GameRunner, ObjectId) {
    let mut sc = GameScenario::new();
    sc.at_phase(Phase::PreCombatMain);
    let sources: Vec<ObjectId> = (0..2)
        .map(|_| {
            sc.add_creature(P0, "Graveyard Adnate", 1, 2)
                .from_oracle_text(SOLDEVI_ADNATE)
                .id()
        })
        .collect();
    sc.add_creature_to_graveyard(P0, "Graveyard Card A", 2, 2)
        .with_mana_cost(ManaCost::generic(4));
    sc.add_creature_to_graveyard(P0, "Graveyard Card B", 2, 2)
        .with_mana_cost(ManaCost::generic(second_mana_value));
    sc.add_creature(P1, "Grizzly Bears", 2, 2);
    let spell = sc
        .add_spell_to_hand_from_oracle(P0, "Eight Black Removal", true, "Destroy target creature.")
        .with_mana_cost(cost(vec![ManaCostShard::Black; 8], 0))
        .id();
    let mut runner = sc.build();
    let exile_cost = AbilityCost::Composite {
        costs: vec![
            AbilityCost::Tap,
            AbilityCost::Exile {
                count: 1,
                zone: Some(Zone::Graveyard),
                filter: Some(TargetFilter::Typed(
                    TypedFilter::card()
                        .controller(ControllerRef::You)
                        .properties(vec![FilterProp::InZone {
                            zone: Zone::Graveyard,
                        }]),
                )),
            },
        ],
    };
    for source in sources {
        let object = runner.state_mut().objects.get_mut(&source).unwrap();
        std::sync::Arc::make_mut(&mut object.abilities)[0].cost = Some(exile_cost.clone());
        std::sync::Arc::make_mut(&mut object.base_abilities)[0].cost = Some(exile_cost.clone());
    }
    (runner, spell)
}

/// Probe R5, offer level (CR 106.1 + CR 601.2h): an output sized by the
/// object its own cost chose is never part of a proven payment. With cards of
/// mana value 4 and 0 the two sources can make at most 4, yet the optimistic
/// planner credits each with 4, so the cast is generated; it is not offered.
/// With two mana value 4 cards it is payable but still unproven (the accepted
/// limit: not offered, though casting it by hand works).
#[test]
fn cost_sized_yield_is_not_a_proven_payment() {
    for second in [0, 4] {
        let (runner, spell) = exile_sized_board(second);
        let action = auto_cast(&runner, spell);
        assert!(
            candidate_actions(runner.state())
                .iter()
                .any(|candidate| candidate.action == action),
            "reach (second card mana value {second}): generated with plain Auto payment"
        );
        assert!(
            !legal_actions(runner.state()).contains(&action),
            "second card mana value {second}: a cost-sized yield proves nothing"
        );
    }
}
