//! Composite costs whose legs use up chosen objects (issue #9784).
//!
//! Jarad, Golgari Lich Lord (Scryfall):
//!   "Sacrifice a Swamp and a Forest: Return this card from your graveyard to your hand."
//! Overgrown Tomb (Scryfall): Land — Swamp Forest.
//! Recurring Nightmare (Scryfall): "Sacrifice a creature, Return this enchantment to its
//!   owner's hand: Return target creature card from your graveyard to the battlefield.
//!   Activate only as a sorcery."
//!
//! CR 118.3: a player can't pay a cost without the resources to pay it fully.
//! CR 601.2h + CR 602.2b: partial payments are not allowed; unpayable costs can't be paid.
//! CR 701.21a: a sacrificed permanent moves to its owner's graveyard, so it can pay only
//! one sacrifice leg.

use engine::game::casting::can_activate_ability_now;
use engine::game::scenario::{GameRunner, GameScenario, P0};
use engine::types::ability::AbilityCost;
use engine::types::actions::GameAction;
use engine::types::card_type::CoreType;
use engine::types::game_state::{PayCostKind, WaitingFor};
use engine::types::identifiers::ObjectId;
use engine::types::mana::ManaColor;
use engine::types::phase::Phase;
use engine::types::zones::Zone;

const JARAD: &str = "Jarad gets +1/+1 for each creature card in your graveyard.\n{1}{B}{G}, Sacrifice another creature: Each opponent loses life equal to the sacrificed creature's power.\nSacrifice a Swamp and a Forest: Return this card from your graveyard to your hand.";
const OVERGROWN_TOMB: &str =
    "({T}: Add {B} or {G}.)\nAs this land enters, you may pay 2 life. If you don't, it enters tapped.";
const RECURRING_NIGHTMARE: &str = "Sacrifice a creature, Return this enchantment to its owner's hand: Return target creature card from your graveyard to the battlefield. Activate only as a sorcery.";

#[derive(Clone, Copy)]
enum Lands {
    /// One Overgrown Tomb.
    TombOnly,
    /// A basic Swamp and a basic Forest.
    BasicPair,
    /// One Overgrown Tomb and a basic Swamp.
    TombAndBasicSwamp,
}

struct JaradBoard {
    runner: GameRunner,
    jarad: ObjectId,
    tomb: Option<ObjectId>,
    basic_swamp: Option<ObjectId>,
    ability: usize,
}

fn jarad_board(lands: Lands) -> JaradBoard {
    let mut scenario = GameScenario::new();
    scenario.at_phase(Phase::PreCombatMain);
    let jarad = scenario
        .add_creature_to_graveyard(P0, "Jarad, Golgari Lich Lord", 2, 2)
        .from_oracle_text(JARAD)
        .id();
    let mut tomb = None;
    let mut basic_swamp = None;
    match lands {
        Lands::TombOnly | Lands::TombAndBasicSwamp => {
            tomb = Some(
                scenario
                    .add_land_from_oracle(P0, "Overgrown Tomb", OVERGROWN_TOMB)
                    .with_subtypes(vec!["Swamp", "Forest"])
                    .id(),
            );
        }
        Lands::BasicPair => {}
    }
    match lands {
        Lands::BasicPair => {
            basic_swamp = Some(scenario.add_basic_land(P0, ManaColor::Black));
            scenario.add_basic_land(P0, ManaColor::Green);
        }
        Lands::TombAndBasicSwamp => {
            basic_swamp = Some(scenario.add_basic_land(P0, ManaColor::Black));
        }
        Lands::TombOnly => {}
    }
    let runner = scenario.build();
    let ability = runner.state().objects[&jarad]
        .abilities
        .iter()
        .position(|a| a.activation_zone == Some(Zone::Graveyard))
        .expect("reach: Jarad's graveyard-return ability is found by its activation zone");
    let cost = runner.state().objects[&jarad].abilities[ability]
        .cost
        .clone();
    let sacrifice_legs = match &cost {
        Some(AbilityCost::Composite { costs }) => costs
            .iter()
            .filter(|c| matches!(c, AbilityCost::Sacrifice(_)))
            .count(),
        _ => 0,
    };
    assert_eq!(
        sacrifice_legs, 2,
        "reach: 'Sacrifice a Swamp and a Forest' parses as a Composite with two Sacrifice legs, got {cost:?}"
    );
    if let Some(tomb) = tomb {
        let subtypes = &runner.state().objects[&tomb].card_types.subtypes;
        assert!(
            subtypes.iter().any(|s| s == "Swamp") && subtypes.iter().any(|s| s == "Forest"),
            "reach: Overgrown Tomb is both a Swamp and a Forest, got {subtypes:?}"
        );
    }
    JaradBoard {
        runner,
        jarad,
        tomb,
        basic_swamp,
        ability,
    }
}

fn sacrifice_choices(runner: &GameRunner) -> Vec<ObjectId> {
    match &runner.state().waiting_for {
        WaitingFor::PayCost {
            kind: PayCostKind::Sacrifice,
            choices,
            ..
        } => choices.clone(),
        other => panic!("expected a sacrifice cost prompt, got {other:?}"),
    }
}

#[test]
fn jarad_graveyard_return_needs_two_distinct_lands() {
    // Control: a separate Swamp and Forest make the ability legal.
    let board = jarad_board(Lands::BasicPair);
    assert!(
        can_activate_ability_now(board.runner.state(), P0, board.jarad, board.ability),
        "reach: with a basic Swamp and a basic Forest the graveyard ability is activatable"
    );

    // One Swamp Forest can pay only one of the two sacrifice legs.
    let board = jarad_board(Lands::TombOnly);
    assert!(
        !can_activate_ability_now(board.runner.state(), P0, board.jarad, board.ability),
        "a single Swamp Forest can't pay 'Sacrifice a Swamp and a Forest' (CR 118.3 / CR 601.2h)"
    );
}

#[test]
fn jarad_single_dual_land_activation_is_refused_without_payment() {
    let JaradBoard {
        mut runner,
        jarad,
        tomb,
        ability,
        ..
    } = jarad_board(Lands::TombOnly);
    let tomb = tomb.unwrap();
    let activated = runner.act(GameAction::ActivateAbility {
        source_id: jarad,
        ability_index: ability,
    });
    assert!(
        activated.is_err(),
        "the activation is refused before any payment, got {activated:?} / {:?}",
        runner.state().waiting_for
    );
    let state = runner.state();
    assert_eq!(state.objects[&jarad].zone, Zone::Graveyard);
    assert_eq!(
        state.objects[&tomb].zone,
        Zone::Battlefield,
        "no partial payment (CR 601.2h): Overgrown Tomb is not sacrificed"
    );
}

/// With an Overgrown Tomb and a basic Swamp the cost is payable (basic Swamp for the
/// Swamp leg, Tomb for the Forest leg). The Swamp leg must not offer the Tomb, or the
/// Forest leg is stranded.
#[test]
fn jarad_swamp_leg_keeps_the_dual_land_for_the_forest_leg() {
    let JaradBoard {
        mut runner,
        jarad,
        tomb,
        basic_swamp,
        ability,
    } = jarad_board(Lands::TombAndBasicSwamp);
    let (tomb, basic_swamp) = (tomb.unwrap(), basic_swamp.unwrap());
    assert!(
        can_activate_ability_now(runner.state(), P0, jarad, ability),
        "a basic Swamp plus a Swamp Forest pays both legs"
    );
    runner
        .act(GameAction::ActivateAbility {
            source_id: jarad,
            ability_index: ability,
        })
        .expect("activation starts");

    assert_eq!(
        sacrifice_choices(&runner),
        vec![basic_swamp],
        "the Swamp leg offers only the basic Swamp; the Tomb is needed for the Forest leg"
    );

    // A forged pick of the Tomb for the Swamp leg is refused with nothing paid.
    let forged = runner.act(GameAction::SelectCards { cards: vec![tomb] });
    assert!(forged.is_err(), "the Tomb is not a legal Swamp-leg choice");
    assert_eq!(runner.state().objects[&tomb].zone, Zone::Battlefield);
    assert_eq!(runner.state().objects[&basic_swamp].zone, Zone::Battlefield);

    runner
        .act(GameAction::SelectCards {
            cards: vec![basic_swamp],
        })
        .expect("the basic Swamp pays the Swamp leg");
    assert_eq!(sacrifice_choices(&runner), vec![tomb]);
    runner
        .act(GameAction::SelectCards { cards: vec![tomb] })
        .expect("the Tomb pays the Forest leg");

    for _ in 0..5 {
        if runner.state().stack.is_empty() {
            break;
        }
        runner.act(GameAction::PassPriority).expect("pass priority");
    }
    let state = runner.state();
    assert_eq!(state.objects[&basic_swamp].zone, Zone::Graveyard);
    assert_eq!(state.objects[&tomb].zone, Zone::Graveyard);
    assert_eq!(
        state.objects[&jarad].zone,
        Zone::Hand,
        "the ability resolves and returns Jarad"
    );
}

struct NightmareBoard {
    runner: GameRunner,
    nightmare: ObjectId,
    bear: Option<ObjectId>,
    ability: usize,
}

/// Recurring Nightmare turned into a creature (an Opalescence-style board) is both a
/// creature it could sacrifice and the permanent its other leg returns to hand.
fn nightmare_board(with_bear: bool) -> NightmareBoard {
    let mut scenario = GameScenario::new();
    scenario.at_phase(Phase::PreCombatMain);
    let nightmare = scenario
        .add_creature(P0, "Recurring Nightmare", 3, 3)
        .from_oracle_text(RECURRING_NIGHTMARE)
        .id();
    let bear = with_bear.then(|| scenario.add_creature(P0, "Grizzly Bears", 2, 2).id());
    scenario.add_creature_to_graveyard(P0, "Gravedigger", 2, 2);
    let mut runner = scenario.build();
    let obj = runner.state_mut().objects.get_mut(&nightmare).unwrap();
    obj.card_types.core_types.push(CoreType::Enchantment);
    obj.base_card_types.core_types.push(CoreType::Enchantment);
    let ability = 0;
    let cost = runner.state().objects[&nightmare].abilities[ability]
        .cost
        .clone();
    let (sacrifice, bounce) = match &cost {
        Some(AbilityCost::Composite { costs }) => (
            costs.iter().any(|c| matches!(c, AbilityCost::Sacrifice(_))),
            costs
                .iter()
                .any(|c| matches!(c, AbilityCost::ReturnToHand { .. })),
        ),
        _ => (false, false),
    };
    assert!(
        sacrifice && bounce,
        "reach: Recurring Nightmare's cost is Composite[Sacrifice, ReturnToHand], got {cost:?}"
    );
    NightmareBoard {
        runner,
        nightmare,
        bear,
        ability,
    }
}

#[test]
fn recurring_nightmare_creature_cant_pay_both_legs_alone() {
    let board = nightmare_board(true);
    assert!(
        can_activate_ability_now(board.runner.state(), P0, board.nightmare, board.ability),
        "reach: with another creature to sacrifice the ability is activatable"
    );
    let board = nightmare_board(false);
    assert!(
        !can_activate_ability_now(board.runner.state(), P0, board.nightmare, board.ability),
        "the Nightmare can't be both the sacrificed creature and the returned enchantment"
    );
}

#[test]
fn recurring_nightmare_sacrifice_leg_excludes_the_returned_permanent() {
    let NightmareBoard {
        mut runner,
        nightmare,
        bear,
        ability,
    } = nightmare_board(true);
    let bear = bear.unwrap();
    runner
        .act(GameAction::ActivateAbility {
            source_id: nightmare,
            ability_index: ability,
        })
        .expect("activation starts");
    // The single graveyard creature card may need an explicit target choice.
    if !matches!(runner.state().waiting_for, WaitingFor::PayCost { .. }) {
        let target = runner
            .state()
            .objects
            .values()
            .find(|o| o.zone == Zone::Graveyard && o.name == "Gravedigger")
            .map(|o| o.id)
            .expect("target card");
        runner
            .act(GameAction::SelectTargets {
                targets: vec![engine::types::ability::TargetRef::Object(target)],
            })
            .expect("target chosen");
    }
    assert_eq!(
        sacrifice_choices(&runner),
        vec![bear],
        "the Nightmare must stay on the battlefield to be returned to hand"
    );
    let forged = runner.act(GameAction::SelectCards {
        cards: vec![nightmare],
    });
    assert!(
        forged.is_err(),
        "sacrificing the Nightmare itself is refused"
    );
    assert_eq!(runner.state().objects[&nightmare].zone, Zone::Battlefield);
}
