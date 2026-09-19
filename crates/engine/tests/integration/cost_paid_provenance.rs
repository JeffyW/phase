//! Measurement suite for the SINGLE cost-paid provenance authority,
//! `ResolvedAbility::cost_paid_objects: Vec<CostPaidObjectSnapshot>`.
//!
//! This replaced a raw `Vec<ObjectId>`. An `ObjectId` is reusable storage
//! identity, so an id alone cannot distinguish "the object this ability's cost
//! moved" from "a different object that later took the same id" (CR 400.7: an
//! object that changes zones becomes a NEW object). The snapshot authority
//! carries the referent's post-cost incarnation so a later consumer can ask
//! that question; these tests measure that the authority is actually produced,
//! and produced LIVE, at every payment seam.
//!
//! Why these tests read internal provenance instead of a board outcome: this
//! change is a deliberate behavioral no-op for today's consumers — the one
//! consumer of the plural authority,
//! `exclude_cost_paid_object_that_left_battlefield`, is membership-only and
//! reads `snapshot.object_id` exactly as it read the raw ids. The
//! observable seam being installed here is the incarnation pin. Every
//! measurement below still drives the REAL pipeline — `GameScenario` +
//! `GameRunner`, real `GameAction` cost payment through the engine's own cost
//! windows — and none hand-constructs a `ResolvedAbility`.
//!
//! Rules riding on these assertions:
//!   * CR 400.7 — a zone change makes a new object; a pin captured BEFORE the
//!     cost's own move names the pre-move object and is stale immediately.
//!   * CR 400.7j — "If the cost of a spell or ability causes an object to move
//!     to a public zone, that spell or ability's effects can find that object."
//!     So the cost's OWN move must not invalidate the reference: each payment
//!     seam re-pins once its moves complete.
//!   * CR 608.2h — the snapshot's `lki` must still record PRE-move
//!     characteristics, which is why capture happens before the move and the
//!     pin is refreshed afterwards rather than the whole snapshot being retaken.
//!   * CR 601.2h / CR 602.2b — the payment is the authority; provenance is
//!     recorded during payment, never reconstructed at resolution.

use engine::game::scenario::{GameRunner, GameScenario, P0, P1};
use engine::types::ability::{CostPaidObjectSnapshot, ResolvedAbility, TargetRef};
use engine::types::actions::GameAction;
use engine::types::game_state::{GameState, PayCostKind, WaitingFor};
use engine::types::identifiers::{ObjectId, LEGACY_INCARNATION};
use engine::types::mana::{ManaCost, ManaType, ManaUnit};
use engine::types::phase::Phase;
use engine::types::zones::Zone;

// ---------------------------------------------------------------------------
// Verbatim Oracle text. Every string below is copied from this repository's own
// existing integration coverage for the same printed card — never paraphrased,
// because a paraphrase can take a different parser branch and go green while
// the real card stays broken.
// ---------------------------------------------------------------------------

/// Harnfel, Horn of Bounty (the Birgi, God of Storytelling back face).
/// Source: `crates/engine/tests/integration/birgi.rs`.
const HARNFEL: &str =
    "Discard a card: Exile the top two cards of your library. You may play those cards this turn.";

/// Pyromancy. Source: `crates/engine/tests/integration/pyromancy_random_discard_cost.rs`.
const PYROMANCY: &str = "{3}, Discard a card at random: Pyromancy deals damage to any target equal to the discarded card's mana value.";

/// Greater Good. Source: `crates/engine/tests/integration/greater_good_activation.rs`.
const GREATER_GOOD: &str = "Sacrifice a creature: Draw cards equal to the sacrificed \
     creature's power, then discard three cards.";

/// Coin of Fate. Source: `crates/engine/tests/integration/coin_of_fate.rs`.
const COIN_OF_FATE: &str = "When this artifact enters, surveil 1.\n{3}{W}, {T}, Exile two creature cards from your graveyard, Sacrifice this artifact: An opponent chooses one of the exiled cards. You put that card on the bottom of your library and return the other to the battlefield tapped. You become the monarch.";

// ---------------------------------------------------------------------------
// Shared drivers
// ---------------------------------------------------------------------------

fn white_pool(count: usize) -> Vec<ManaUnit> {
    (0..count)
        .map(|_| ManaUnit::new(ManaType::White, ObjectId(0), false, vec![]))
        .collect()
}

fn colorless_pool(count: usize) -> Vec<ManaUnit> {
    (0..count)
        .map(|_| ManaUnit::new(ManaType::Colorless, ObjectId(0), false, vec![]))
        .collect()
}

/// The index of the single costed activated ability on `source`.
fn costed_ability_index(runner: &GameRunner, source: ObjectId) -> usize {
    runner.state().objects[&source]
        .abilities
        .iter()
        .position(|ability| ability.cost.is_some())
        .expect("the fixture card must carry an activated ability with a cost")
}

/// `object`'s live incarnation epoch right now. Recorded BEFORE payment so the
/// tests can prove the cost's own move actually advanced it — without that, a
/// snapshot that was never re-pinned would still compare equal to the live
/// object and the measurement would be vacuous.
fn incarnation(runner: &GameRunner, object: ObjectId) -> u64 {
    runner.state().objects[&object].incarnation
}

/// Answer the engine's own cost windows until the activation reaches the stack.
///
/// `cost_cards` are the objects the caller intends to pay the non-mana cost
/// with; a `PayCost` window whose eligible set does not contain them (a
/// self-sacrifice leg, for instance) is answered from its own `choices`, so
/// this driver never guesses which cost leg it is looking at.
fn pay_until_on_stack(runner: &mut GameRunner, cost_cards: &[ObjectId]) {
    for _ in 0..24 {
        if !runner.state().stack.is_empty() {
            return;
        }
        match runner.state().waiting_for.clone() {
            WaitingFor::PayCost { choices, count, .. } => {
                let mut selection: Vec<ObjectId> = cost_cards
                    .iter()
                    .copied()
                    .filter(|id| choices.contains(id))
                    .collect();
                if selection.len() != count {
                    selection = choices.iter().copied().take(count).collect();
                }
                runner
                    .act(GameAction::SelectCards { cards: selection })
                    .expect("the engine's own cost window must accept its own eligible set");
            }
            WaitingFor::ManaPayment { .. } => {
                runner
                    .act(GameAction::PassPriority)
                    .expect("the mana cost must finalize from the floating pool");
            }
            other => panic!("the activation stalled before reaching the stack at {other:?}"),
        }
    }
    panic!("the activation never reached the stack within the bounded driver");
}

/// The resolving ability sitting on the stack, i.e. the carrier that owns the
/// cost-paid provenance the payment just published.
fn ability_on_stack(runner: &GameRunner) -> &ResolvedAbility {
    let state = runner.state();
    let entry = state.stack.last().unwrap_or_else(|| {
        panic!(
            "reach guard: the activation must be on the stack; waiting_for={:?}",
            state.waiting_for
        )
    });
    entry
        .ability()
        .expect("an activated-ability stack entry carries its ResolvedAbility")
}

/// The object-id projection of the plural authority, in payment order. This is
/// exactly what the Phase-1 compatibility consumers read.
fn paid_ids(ability: &ResolvedAbility) -> Vec<ObjectId> {
    ability
        .cost_paid_objects
        .iter()
        .map(|snapshot| snapshot.object_id)
        .collect()
}

/// The core measurement: one plural snapshot names `expected_id`, that object
/// really moved (reach guard), the move really advanced its incarnation, and
/// the snapshot was RE-PINNED so it still resolves live (CR 400.7j).
///
/// Revert sensitivity lives in the last two assertions: delete the payment
/// seam's `repin_cost_paid_object_recursive` call and the snapshot keeps its
/// pre-move epoch, so `snapshot.incarnation == live.incarnation` fails and
/// `live_object_id` yields `None` instead of `Some(id)`.
fn assert_repinned_live(
    state: &GameState,
    snapshot: &CostPaidObjectSnapshot,
    expected_id: ObjectId,
    expected_zone: Zone,
    incarnation_before_payment: u64,
    label: &str,
) {
    assert_eq!(
        snapshot.object_id, expected_id,
        "{label}: the snapshot must name the object this cost consumed"
    );
    let live = state.objects.get(&expected_id).unwrap_or_else(|| {
        panic!("{label}: the cost-paid object's row must survive its own cost move")
    });
    assert_eq!(
        live.zone, expected_zone,
        "{label}: reach guard — the cost's own move actually happened"
    );
    assert_ne!(
        live.incarnation, incarnation_before_payment,
        "{label}: CR 400.7 — the cost's own move must make a new object, otherwise \
         this measurement cannot tell a re-pinned snapshot from a stale one"
    );
    assert_eq!(
        snapshot.incarnation, live.incarnation,
        "{label}: CR 400.7j — the snapshot must be re-pinned to the incarnation the \
         cost's OWN move produced, not left on its pre-move capture"
    );
    assert_eq!(
        snapshot.live_object_id(state),
        Some(expected_id),
        "{label}: CR 400.7j — the cost-paid referent must resolve LIVE immediately \
         after its own cost payment"
    );
}

// ---------------------------------------------------------------------------
// Deterministic discard — Harnfel, Horn of Bounty
// ---------------------------------------------------------------------------

/// CR 701.9a + CR 400.7j: a discard cost moves the card to the graveyard — a
/// public zone — so this same ability's effects can still find it. The pin must
/// therefore survive the cost's own move.
///
/// Fixture non-degeneracy: TWO cards sit in hand, so the discard window offers a
/// real choice and the payment is not the "only possible card" branch.
#[test]
fn deterministic_discard_cost_publishes_a_live_snapshot() {
    let mut scenario = GameScenario::new();
    scenario.at_phase(Phase::PreCombatMain);
    let harnfel = scenario
        .add_artifact_from_oracle(P0, "Harnfel, Horn of Bounty", HARNFEL)
        .id();
    let paid = scenario
        .add_creature_to_hand(P0, "Discard Fodder", 2, 2)
        .id();
    let kept = scenario.add_creature_to_hand(P0, "Kept Card", 3, 3).id();
    scenario.with_library_top(P0, &["L1", "L2", "L3"]);
    let mut runner = scenario.build();

    let before = incarnation(&runner, paid);
    let index = costed_ability_index(&runner, harnfel);
    runner
        .act(GameAction::ActivateAbility {
            source_id: harnfel,
            ability_index: index,
        })
        .expect("Harnfel's discard ability must be activatable");
    assert!(
        matches!(
            runner.state().waiting_for,
            WaitingFor::PayCost {
                kind: PayCostKind::Discard,
                ..
            }
        ),
        "reach guard: the real discard cost window must open, got {:?}",
        runner.state().waiting_for
    );
    runner
        .act(GameAction::SelectCards { cards: vec![paid] })
        .expect("discarding an eligible hand card must pay the cost");
    pay_until_on_stack(&mut runner, &[paid]);

    let ability = ability_on_stack(&runner);
    assert_eq!(
        paid_ids(ability),
        vec![paid],
        "the plural authority records exactly the card the cost discarded"
    );
    assert_repinned_live(
        runner.state(),
        &ability.cost_paid_objects[0],
        paid,
        Zone::Graveyard,
        before,
        "deterministic discard",
    );

    // Sibling case: the card that was NOT paid is untouched and is not recorded.
    assert_eq!(
        runner.state().objects[&kept].zone,
        Zone::Hand,
        "the unpaid hand card stays in hand"
    );
    assert!(
        !paid_ids(ability).contains(&kept),
        "only objects the cost actually consumed enter the authority"
    );
}

// ---------------------------------------------------------------------------
// Random discard — Pyromancy
// ---------------------------------------------------------------------------

/// CR 701.9b + CR 400.7j: a RANDOM discard cost publishes provenance from the
/// payment's own `RandomDiscardCostPick::snapshot`, captured by the random
/// discard routine before the card left the hand. `commit_random_discard_cost_picks`
/// consumes those snapshots directly and re-pins them; it never reconstructs
/// provenance from an id.
///
/// Fixture non-degeneracy: TWO hand cards, so the seeded RNG genuinely selects
/// one of them and the test does not assume which.
#[test]
fn random_discard_cost_publishes_a_live_snapshot() {
    let mut scenario = GameScenario::new();
    scenario.at_phase(Phase::PreCombatMain);
    scenario.with_mana_pool(P0, colorless_pool(3));
    let pyromancy = scenario
        .add_enchantment_from_oracle(P0, "Pyromancy", PYROMANCY)
        .id();
    let hand: Vec<ObjectId> = (0..2)
        .map(|i| {
            scenario
                .add_creature_to_hand(P0, &format!("Random Filler {i}"), 1, 1)
                .with_mana_cost(ManaCost::generic(3))
                .id()
        })
        .collect();
    let mut runner = scenario.build();

    let before: Vec<(ObjectId, u64)> = hand
        .iter()
        .map(|&id| (id, incarnation(&runner, id)))
        .collect();

    runner
        .act(GameAction::ActivateAbility {
            source_id: pyromancy,
            ability_index: 0,
        })
        .expect("Pyromancy's random-discard ability must be activatable");
    runner
        .act(GameAction::ChooseTarget {
            target: Some(TargetRef::Player(P1)),
        })
        .expect("Pyromancy targets any target");
    pay_until_on_stack(&mut runner, &hand);

    let discarded: Vec<ObjectId> = hand
        .iter()
        .copied()
        .filter(|id| runner.state().objects[id].zone == Zone::Graveyard)
        .collect();
    assert_eq!(
        discarded.len(),
        1,
        "reach guard: the production RNG path must have discarded exactly one card"
    );
    let discarded = discarded[0];
    let before = before
        .iter()
        .find(|(id, _)| *id == discarded)
        .map(|(_, epoch)| *epoch)
        .expect("the discarded card is one of the seeded hand cards");

    let ability = ability_on_stack(&runner);
    assert_eq!(
        paid_ids(ability),
        vec![discarded],
        "the plural authority records exactly the randomly discarded card"
    );
    assert_repinned_live(
        runner.state(),
        &ability.cost_paid_objects[0],
        discarded,
        Zone::Graveyard,
        before,
        "random discard",
    );
}

// ---------------------------------------------------------------------------
// Sacrifice — Greater Good
// ---------------------------------------------------------------------------

/// CR 701.21a + CR 400.7j: a sacrifice cost moves the permanent to its owner's
/// graveyard, so the same publication/re-pin contract applies.
///
/// A NONTOKEN victim is deliberate: a sacrificed token ceases to exist
/// (CR 704.5d) and is purged from `state.objects`, which would make "the pin
/// still resolves live" unmeasurable rather than false.
///
/// Fixture non-degeneracy: TWO eligible creatures, so the sacrifice window is a
/// real choice.
#[test]
fn sacrifice_cost_publishes_a_live_snapshot() {
    let mut scenario = GameScenario::new();
    scenario.at_phase(Phase::PreCombatMain);
    let good = scenario
        .add_creature(P0, "Greater Good", 0, 0)
        .from_oracle_text(GREATER_GOOD)
        .as_enchantment()
        .id();
    let victim = scenario.add_creature(P0, "Beast", 5, 5).id();
    let survivor = scenario.add_creature(P0, "Bystander", 2, 2).id();
    scenario.with_library_top(P0, &["L1", "L2", "L3", "L4", "L5", "L6"]);
    scenario.with_cards_in_hand(P0, &["H1", "H2", "H3"]);
    let mut runner = scenario.build();

    let before = incarnation(&runner, victim);
    let index = costed_ability_index(&runner, good);
    runner
        .act(GameAction::ActivateAbility {
            source_id: good,
            ability_index: index,
        })
        .expect("Greater Good's sacrifice ability must be activatable");
    assert!(
        matches!(
            runner.state().waiting_for,
            WaitingFor::PayCost {
                kind: PayCostKind::Sacrifice,
                ..
            }
        ),
        "reach guard: the real sacrifice cost window must open, got {:?}",
        runner.state().waiting_for
    );
    runner
        .act(GameAction::SelectCards {
            cards: vec![victim],
        })
        .expect("sacrificing an eligible creature must pay the cost");
    pay_until_on_stack(&mut runner, &[victim]);

    let ability = ability_on_stack(&runner);
    assert_eq!(
        paid_ids(ability),
        vec![victim],
        "the plural authority records exactly the sacrificed permanent"
    );
    assert_repinned_live(
        runner.state(),
        &ability.cost_paid_objects[0],
        victim,
        Zone::Graveyard,
        before,
        "sacrifice",
    );

    // Sibling case: an equally eligible permanent that was not chosen is
    // neither moved nor recorded.
    assert_eq!(
        runner.state().objects[&survivor].zone,
        Zone::Battlefield,
        "the unchosen creature stays on the battlefield"
    );
    assert!(
        !paid_ids(ability).contains(&survivor),
        "only objects the cost actually consumed enter the authority"
    );
}

// ---------------------------------------------------------------------------
// Exile — Coin of Fate (a cost move into a public zone)
// ---------------------------------------------------------------------------

struct CoinBoard {
    runner: GameRunner,
    grave_a: ObjectId,
    grave_b: ObjectId,
    /// A third graveyard creature card that is eligible but not selected, so
    /// the two-card exile cost is a genuine choice rather than a forced set.
    grave_c: ObjectId,
}

fn coin_board() -> CoinBoard {
    let mut scenario = GameScenario::new();
    scenario.at_phase(Phase::PreCombatMain);
    scenario.with_mana_pool(P0, white_pool(4));
    let coin = scenario
        .add_artifact_from_oracle(P0, "Coin of Fate", COIN_OF_FATE)
        .id();
    let grave_a = scenario
        .add_creature_to_graveyard(P0, "Graveyard Creature A", 2, 2)
        .id();
    let grave_b = scenario
        .add_creature_to_graveyard(P0, "Graveyard Creature B", 3, 3)
        .id();
    let grave_c = scenario
        .add_creature_to_graveyard(P0, "Graveyard Creature C", 4, 4)
        .id();
    scenario.add_card_to_library_top(P0, "Library Filler");
    let mut runner = scenario.build();

    let index = costed_ability_index(&runner, coin);
    runner
        .act(GameAction::ActivateAbility {
            source_id: coin,
            ability_index: index,
        })
        .expect("Coin of Fate's ability must be activatable with the cost available");
    CoinBoard {
        runner,
        grave_a,
        grave_b,
        grave_c,
    }
}

/// CR 701.13a + CR 400.7j: Coin's cost exiles two graveyard creature cards.
/// Exile is a public zone, so CR 400.7j is exactly the rule that lets Coin's own
/// effect ("An opponent chooses one of the exiled cards") find them — which is
/// why this seam is the one a live-incarnation consumer will later resolve
/// through. Both entries must be re-pinned, and their payment ORDER preserved.
#[test]
fn exile_cost_publishes_live_snapshots_in_payment_order() {
    let mut board = coin_board();
    let (grave_a, grave_b, grave_c) = (board.grave_a, board.grave_b, board.grave_c);
    let before_a = incarnation(&board.runner, grave_a);
    let before_b = incarnation(&board.runner, grave_b);

    pay_until_on_stack(&mut board.runner, &[grave_a, grave_b]);

    let ability = ability_on_stack(&board.runner);
    let ids = paid_ids(ability);
    let cost_pair: Vec<ObjectId> = ids
        .iter()
        .copied()
        .filter(|id| *id == grave_a || *id == grave_b)
        .collect();
    assert_eq!(
        cost_pair,
        vec![grave_a, grave_b],
        "CR 601.2h: both exiled cards are recorded, in the order they were paid; \
         full authority = {ids:?}"
    );
    assert!(
        !ids.contains(&grave_c),
        "the eligible-but-unselected graveyard card was not consumed by this cost"
    );

    for (id, before, label) in [
        (grave_a, before_a, "exile (first paid)"),
        (grave_b, before_b, "exile (second paid)"),
    ] {
        let snapshot = ability
            .cost_paid_objects
            .iter()
            .find(|snapshot| snapshot.object_id == id)
            .expect("each exiled card has its own snapshot");
        assert_repinned_live(
            board.runner.state(),
            snapshot,
            id,
            Zone::Exile,
            before,
            label,
        );
    }
}

// ---------------------------------------------------------------------------
// Equality contract — the replacement must not narrow `ResolvedAbility` identity
// ---------------------------------------------------------------------------

/// The field this replaced was a raw `Vec<ObjectId>`, whose equality compared
/// exactly the ids. `CostPaidObjectSnapshot` derives a FULL `PartialEq`, so a
/// mechanical swap would have folded `lki` and `incarnation` into every identity
/// check that reaches `ResolvedAbility` equality — `GameState`'s own
/// `PartialEq` over `stack`/`waiting_for`, and stack copy/batch run identity
/// most of all. `ResolvedAbility`'s manual `PartialEq` compares this field by
/// object-id sequence only, preserving the previous semantics exactly.
///
/// Revert sensitivity: restoring a plain `a == b` vector comparison (or deriving
/// `PartialEq`) makes the first assertion fail, because the refreshed clone
/// differs only in the pins.
#[test]
fn plural_cost_paid_identity_is_the_object_id_sequence_only() {
    let mut board = coin_board();
    let (grave_a, grave_b) = (board.grave_a, board.grave_b);
    pay_until_on_stack(&mut board.runner, &[grave_a, grave_b]);
    let ability = ability_on_stack(&board.runner).clone();
    assert!(
        ability.cost_paid_objects.len() >= 2,
        "reach guard: this measurement needs a real multi-object payment, got {:?}",
        paid_ids(&ability)
    );

    let mut repinned = ability.clone();
    for snapshot in repinned.cost_paid_objects.iter_mut() {
        snapshot.incarnation = snapshot.incarnation.wrapping_add(1);
    }
    assert_eq!(
        ability, repinned,
        "two abilities whose costs consumed the same objects in the same order are \
         the same ability; differing pins must not narrow identity"
    );

    let mut reordered = ability.clone();
    reordered.cost_paid_objects.reverse();
    assert_ne!(
        ability, reordered,
        "a DIFFERENT object-id sequence is a different ability — the id-sequence \
         comparison must not be vacuously true"
    );
}

// ---------------------------------------------------------------------------
// Restore contract — missing provenance fails closed
// ---------------------------------------------------------------------------

/// `ResolvedAbility` reaches the wire through `GameState` restore / P2P paths.
/// A `CostPaidObjectSnapshot` written before the incarnation epoch existed
/// cannot prove which incarnation it bound, so serde defaults it to
/// `LEGACY_INCARNATION`, which no live object can ever match: the record reads
/// as stale rather than silently naming whatever object holds the id today
/// (CR 400.7). Measured on a REAL snapshot produced by a real cost payment.
#[test]
fn a_legacy_snapshot_without_an_incarnation_fails_closed() {
    let mut board = coin_board();
    let (grave_a, grave_b) = (board.grave_a, board.grave_b);
    pay_until_on_stack(&mut board.runner, &[grave_a, grave_b]);
    let ability = ability_on_stack(&board.runner).clone();
    let live = ability
        .cost_paid_objects
        .iter()
        .find(|snapshot| snapshot.object_id == grave_a)
        .expect("the cost-exiled card has a snapshot")
        .clone();
    // Reach guard: the record we are about to downgrade really is live now, so
    // the fail-closed assertion below cannot pass for the wrong reason.
    assert_eq!(
        live.live_object_id(board.runner.state()),
        Some(grave_a),
        "the freshly paid snapshot must resolve live before it is downgraded"
    );

    let mut json = serde_json::to_value(&live).expect("a cost-paid snapshot serializes");
    json.as_object_mut()
        .expect("a snapshot serializes as a JSON object")
        .remove("incarnation");
    let legacy: CostPaidObjectSnapshot =
        serde_json::from_value(json).expect("a pre-migration snapshot must still restore");

    assert_eq!(
        legacy.object_id, grave_a,
        "the legacy record still carries its storage id"
    );
    assert_eq!(
        legacy.incarnation, LEGACY_INCARNATION,
        "a record with no captured epoch is pinned to the sentinel"
    );
    assert!(
        !legacy.is_current(board.runner.state()),
        "CR 400.7: the sentinel can never match a live object"
    );
    assert_eq!(
        legacy.live_object_id(board.runner.state()),
        None,
        "fail closed: a legacy record must resolve to nothing rather than rebind by storage id"
    );
}

/// The other half of the restore contract: a persisted ability that predates
/// the snapshot authority carries a raw `cost_paid_object_ids` id list. Those
/// ids must be IGNORED, never rebound into snapshots — the same reason as
/// above, one level up. Measured on a REAL resolving ability.
#[test]
fn a_legacy_raw_id_payload_never_rebinds_the_authority() {
    let mut board = coin_board();
    let (grave_a, grave_b) = (board.grave_a, board.grave_b);
    pay_until_on_stack(&mut board.runner, &[grave_a, grave_b]);
    let ability = ability_on_stack(&board.runner).clone();
    let live_ids = paid_ids(&ability);
    assert!(
        !live_ids.is_empty(),
        "reach guard: the ability must actually carry provenance to downgrade"
    );

    let mut json = serde_json::to_value(&ability).expect("a resolved ability serializes");
    let object = json
        .as_object_mut()
        .expect("a resolved ability serializes as a JSON object");
    assert!(
        object.contains_key("cost_paid_objects"),
        "a populated authority is written to the wire"
    );
    object.remove("cost_paid_objects");
    object.insert(
        "cost_paid_object_ids".to_string(),
        serde_json::Value::Array(
            live_ids
                .iter()
                .map(|id| serde_json::json!(id.0))
                .collect::<Vec<_>>(),
        ),
    );

    let restored: ResolvedAbility =
        serde_json::from_value(json).expect("a pre-migration ability payload must still restore");
    assert!(
        restored.cost_paid_objects.is_empty(),
        "legacy raw ids must not be rebound into the snapshot authority; got {:?}",
        paid_ids(&restored)
    );
}
