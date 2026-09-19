//! Coin of Fate (Artifact {1}{W}) — the source-bound cost-paid exile partition.
//!
//! ```text
//! When this artifact enters, surveil 1.
//! {3}{W}, {T}, Exile two creature cards from your graveyard, Sacrifice this
//! artifact: An opponent chooses one of the exiled cards. You put that card on
//! the bottom of your library and return the other to the battlefield tapped.
//! You become the monarch.
//! ```
//!
//! Every test here drives the REAL pipeline — `GameAction::ActivateAbility` →
//! the engine's own cost windows (`WaitingFor::PayCost` for the graveyard exile,
//! `WaitingFor::ManaPayment` for `{3}{W}`) → stack resolution →
//! `WaitingFor::ChooseFromZoneChoice` answered with a real
//! `GameAction::SelectCards` — and asserts only OBSERVABLE outcomes: which
//! player is prompted, which cards are offered, and where every object ends up.
//! No AST shapes are asserted; the parser-level guards for this change live in
//! `crates/engine/src/parser/oracle_effect/imperative.rs`'s unit tests.
//!
//! Two rules ride on these assertions:
//!   * CR 400.7j + CR 601.2h + CR 602.2b — the activation cost moved two cards
//!     to exile (a public zone), so this same ability's effect can find exactly
//!     those two and nothing else. The board deliberately carries an UNRELATED
//!     creature card already sitting in exile: a candidate pool that scanned the
//!     exile zone instead of the cost-payment record would offer it.
//!   * CR 608.2c + CR 608.2d — "that card" is the opponent's pick and "the
//!     other" is its complement. The engine forwards the CHOSEN cards as the
//!     continuation's targets and the UNCHOSEN complement only on the
//!     continuation's immediate sub-ability, so the two halves must land in
//!     different zones.

use engine::game::scenario::{GameRunner, GameScenario, P0, P1};
use engine::types::actions::GameAction;
use engine::types::game_state::WaitingFor;
use engine::types::identifiers::ObjectId;
use engine::types::mana::{ManaType, ManaUnit};
use engine::types::phase::Phase;
use engine::types::player::PlayerId;
use engine::types::zones::Zone;

/// Verbatim Oracle text (engine card-data export). Never paraphrase: a
/// paraphrase can take a different parser branch and go green while the real
/// card stays broken.
const COIN_OF_FATE: &str = "When this artifact enters, surveil 1.\n{3}{W}, {T}, Exile two creature cards from your graveyard, Sacrifice this artifact: An opponent chooses one of the exiled cards. You put that card on the bottom of your library and return the other to the battlefield tapped. You become the monarch.";

/// The board under test.
struct Board {
    runner: GameRunner,
    coin: ObjectId,
    /// The two creature cards in P0's graveyard that pay the exile cost.
    grave_a: ObjectId,
    grave_b: ObjectId,
    /// A creature card ALREADY in exile before activation — the hostile fixture
    /// that separates "the cards this cost exiled" from "the cards in exile".
    stale_exile: ObjectId,
    /// A card seeded into P0's library so "bottom of your library" is a real
    /// position rather than a one-element degenerate case.
    library_card: ObjectId,
}

fn board() -> Board {
    let mut scenario = GameScenario::new();
    scenario.at_phase(Phase::PreCombatMain);
    // {3}{W}: four white mana covers the white pip and the three generic.
    scenario.with_mana_pool(
        P0,
        (0..4)
            .map(|_| ManaUnit::new(ManaType::White, ObjectId(0), false, vec![]))
            .collect(),
    );
    let coin = scenario
        .add_artifact_from_oracle(P0, "Coin of Fate", COIN_OF_FATE)
        .id();
    let grave_a = scenario
        .add_creature_to_graveyard(P0, "Graveyard Creature A", 2, 2)
        .id();
    let grave_b = scenario
        .add_creature_to_graveyard(P0, "Graveyard Creature B", 3, 3)
        .id();
    // Same owner, same zone, same card type as the cost-exiled pair — the only
    // thing that distinguishes it is that Coin's cost did not put it there.
    let stale_exile = scenario
        .add_creature_to_exile(P0, "Unrelated Exiled Creature", 4, 4)
        .id();
    let library_card = scenario.add_card_to_library_top(P0, "Library Filler");
    let runner = scenario.build();
    Board {
        runner,
        coin,
        grave_a,
        grave_b,
        stale_exile,
        library_card,
    }
}

/// Index of Coin's single activated ability (the only one carrying a cost).
fn activated_ability_index(runner: &GameRunner, coin: ObjectId) -> usize {
    runner.state().objects[&coin]
        .abilities
        .iter()
        .position(|a| a.cost.is_some())
        .expect("Coin of Fate must carry an activated ability with a cost")
}

/// Announce the activation and answer every cost/priority window the engine
/// raises until the resolution-time choice opens.
///
/// `cost_cards` are the objects the caller intends to pay the non-mana cost
/// with. Any `PayCost` window whose eligible set does not contain them (the
/// self-sacrifice leg, should the engine surface it) is answered from its own
/// `choices`, so this driver never guesses which cost leg it is looking at.
fn activate_until_choice(board: &mut Board, cost_cards: &[ObjectId]) {
    let index = activated_ability_index(&board.runner, board.coin);
    board
        .runner
        .act(GameAction::ActivateAbility {
            source_id: board.coin,
            ability_index: index,
        })
        .expect("Coin's ability must be activatable with the cost available");

    for _ in 0..40 {
        match board.runner.state().waiting_for.clone() {
            WaitingFor::ChooseFromZoneChoice { .. } => return,
            WaitingFor::PayCost { choices, count, .. } => {
                let mut selection: Vec<ObjectId> = cost_cards
                    .iter()
                    .copied()
                    .filter(|id| choices.contains(id))
                    .collect();
                if selection.len() != count {
                    selection = choices.iter().copied().take(count).collect();
                }
                board
                    .runner
                    .act(GameAction::SelectCards { cards: selection })
                    .expect("cost payment must be accepted");
            }
            WaitingFor::ManaPayment { .. } => {
                board
                    .runner
                    .act(GameAction::PassPriority)
                    .expect("the mana cost must finalize from the floating pool");
            }
            WaitingFor::Priority { .. } => {
                if board.runner.act(GameAction::PassPriority).is_err() {
                    return;
                }
            }
            _ => return,
        }
    }
}

/// The offered pool at the open zone choice, plus the player being asked.
fn open_choice(runner: &GameRunner) -> (PlayerId, Vec<ObjectId>) {
    match &runner.state().waiting_for {
        WaitingFor::ChooseFromZoneChoice { player, cards, .. } => (*player, cards.to_vec()),
        other => panic!(
            "expected the resolution-time ChooseFromZoneChoice to be open, got {other:?}; \
             the ability never reached its choice"
        ),
    }
}

// ---------------------------------------------------------------------------
// Claim 1 — the candidate pool is source-bound to the cost payment, and the
// OPPONENT is the one asked.
// ---------------------------------------------------------------------------

/// CR 400.7j + CR 608.2d: the prompt offers EXACTLY the two creature cards the
/// activation cost exiled — not the unrelated creature card that was already in
/// exile, and not the sacrificed Coin (which the same cost recorded but which
/// went to the graveyard, not exile).
///
/// CR 608.2d: "An opponent chooses" — the prompt goes to P1, not to Coin's
/// controller.
///
/// Reach guard: the assertion that the wait IS `ChooseFromZoneChoice` (inside
/// `open_choice`) proves the activation, the cost payment and the resolution all
/// happened; a negative-only "the stale card is absent" assertion would pass
/// vacuously if the ability never resolved.
#[test]
fn coin_of_fate_offers_only_the_cost_exiled_pair_to_an_opponent() {
    let mut board = board();
    let (grave_a, grave_b, stale, coin) =
        (board.grave_a, board.grave_b, board.stale_exile, board.coin);
    activate_until_choice(&mut board, &[grave_a, grave_b]);

    let (player, cards) = open_choice(&board.runner);
    assert_eq!(
        player, P1,
        "CR 608.2d: 'An opponent chooses' — the opponent is prompted, not Coin's controller"
    );
    assert_eq!(
        cards.len(),
        2,
        "exactly the two cost-exiled cards are candidates, got {cards:?}"
    );
    assert!(
        cards.contains(&grave_a) && cards.contains(&grave_b),
        "both cost-exiled creature cards must be offered, got {cards:?}"
    );
    assert!(
        !cards.contains(&stale),
        "CR 400.7j: a creature card already in exile was NOT moved there by this \
         ability's cost, so it is not one of 'the exiled cards'"
    );
    assert!(
        !cards.contains(&coin),
        "the sacrificed Coin is a cost-paid object but it is in the graveyard, not exile"
    );

    // Provenance cross-check on the same board: the stale card really is in
    // exile, so its absence above is a source binding and not an empty zone.
    assert_eq!(
        board.runner.state().objects[&stale].zone,
        Zone::Exile,
        "the hostile fixture must actually be sitting in exile for its exclusion to mean anything"
    );
}

// ---------------------------------------------------------------------------
// Claim 2 — "that card" and "the other" bind to complementary halves.
// ---------------------------------------------------------------------------

/// CR 608.2c: the CHOSEN card goes to the bottom of the controller's library and
/// the UNCHOSEN one returns to the battlefield tapped — the two instructions
/// name different objects, so a binding that returned the chosen card would put
/// both halves in the wrong zone. CR 725.1: the controller becomes the monarch.
#[test]
fn coin_of_fate_bottoms_the_chosen_card_and_returns_the_other_tapped() {
    let mut board = board();
    let (grave_a, grave_b, stale, coin, filler) = (
        board.grave_a,
        board.grave_b,
        board.stale_exile,
        board.coin,
        board.library_card,
    );
    activate_until_choice(&mut board, &[grave_a, grave_b]);

    let (_, cards) = open_choice(&board.runner);
    assert!(
        cards.contains(&grave_a),
        "reach guard: the card this test selects must actually be on offer"
    );
    board
        .runner
        .act(GameAction::SelectCards {
            cards: vec![grave_a],
        })
        .expect("the opponent's pick must be accepted");

    let state = board.runner.state();

    // Positive reach guard FIRST: the chosen half actually moved to the library.
    assert_eq!(
        state.objects[&grave_a].zone,
        Zone::Library,
        "CR 608.2c: the chosen card is put into the controller's library"
    );
    let library = &state
        .players
        .iter()
        .find(|p| p.id == P0)
        .expect("P0")
        .library;
    assert_eq!(
        library.last().copied(),
        Some(grave_a),
        "the chosen card goes on the BOTTOM of the library (library: {library:?})"
    );
    assert!(
        library.iter().any(|id| *id == filler),
        "the pre-seeded library card must still be there, so 'bottom' is a real position"
    );

    // The complement — the half the opponent did NOT pick.
    assert_eq!(
        state.objects[&grave_b].zone,
        Zone::Battlefield,
        "CR 608.2c: 'the other' — the UNCHOSEN card returns to the battlefield"
    );
    assert!(
        state.objects[&grave_b].tapped,
        "'return the other to the battlefield tapped' — it enters tapped"
    );

    // The halves are disjoint: neither ended up where the other belongs.
    assert_ne!(
        state.objects[&grave_a].zone,
        Zone::Battlefield,
        "the chosen card must NOT be the one returned to the battlefield"
    );
    assert_ne!(
        state.objects[&grave_b].zone,
        Zone::Library,
        "the unchosen card must NOT be the one put on the bottom of the library"
    );

    // Nothing else moved, and the monarch designation landed.
    assert_eq!(
        state.objects[&stale].zone,
        Zone::Exile,
        "the unrelated exiled card is untouched by this resolution"
    );
    assert_eq!(
        state.objects[&coin].zone,
        Zone::Graveyard,
        "Coin sacrificed itself to pay its own activation cost"
    );
    assert_eq!(
        state.monarch,
        Some(P0),
        "CR 725.1: 'You become the monarch' — Coin's controller"
    );
}

/// The complement binding is symmetric: picking the OTHER card swaps which half
/// is bottomed and which returns. This is the sibling case for the test above —
/// without it, a binding that happened to always name `grave_b` for the return
/// would satisfy a single-direction assertion by coincidence.
#[test]
fn coin_of_fate_partition_follows_the_opponents_pick_either_way() {
    let mut board = board();
    let (grave_a, grave_b) = (board.grave_a, board.grave_b);
    activate_until_choice(&mut board, &[grave_a, grave_b]);

    let (_, cards) = open_choice(&board.runner);
    assert!(
        cards.contains(&grave_b),
        "reach guard: the card this test selects must actually be on offer"
    );
    board
        .runner
        .act(GameAction::SelectCards {
            cards: vec![grave_b],
        })
        .expect("the opponent's pick must be accepted");

    let state = board.runner.state();
    let library = &state
        .players
        .iter()
        .find(|p| p.id == P0)
        .expect("P0")
        .library;
    assert_eq!(
        library.last().copied(),
        Some(grave_b),
        "picking B bottoms B (library: {library:?})"
    );
    assert_eq!(
        state.objects[&grave_a].zone,
        Zone::Battlefield,
        "picking B returns A to the battlefield"
    );
    assert!(
        state.objects[&grave_a].tapped,
        "the returned card enters tapped regardless of which half it is"
    );
}
