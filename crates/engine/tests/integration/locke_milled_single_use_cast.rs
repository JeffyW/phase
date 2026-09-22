//! CR 601.2a + CR 603.7 + CR 608.2g + CR 611.2a: Locke, Treasure Hunter — a
//! duration-scoped cast permission over a MILLED batch, capped at one.
//!
//! Locke is the first shipped card to pair a non-exile pool with a serialized
//! `single_use` grant, and it reached three separate gaps that each looked local
//! and were not:
//!
//!   1. **Routing.** The clause lowered to
//!      `Unimplemented { name: "unrepresentable_cast_cap" }`. The `from among`
//!      batch authority selects `CastMechanism::LingeringPermission` for a PAID
//!      cast, and `for_batch_bounds` refuses that mechanism a printed cap of one
//!      because it records an INDEPENDENT permission per object with no shared
//!      budget. The shape that does carry a grant-scoped budget of one already
//!      existed — `CastingPermission::PlayFromExile { single_use: true }`, which
//!      Chandra, Hope's Beacon +1 has used since it shipped — and the batch
//!      surfaces simply had no route to it.
//!
//!   2. **The cap was unenforceable off-exile.** The capture that writes the
//!      spent-grant ledger was gated on `source_zone == Zone::Exile`
//!      (`casting_costs.rs`), while the eligibility gate that READS the ledger
//!      (`play_from_exile_permission_source_at_index`) is zone-agnostic. A
//!      graveyard-sourced cast therefore never wrote the ledger and the gate kept
//!      passing, so a grant printing "a spell" authorized a SECOND one.
//!
//!   3. **The sibling sweep was zone-blind in the wrong direction.**
//!      `consume_single_use_play_from_exile` iterated `state.exile` alone, so
//!      milled siblings sitting in graveyards kept a permission the engine had
//!      just declared spent.
//!
//! Fixes 2 and 3 are only observable once 1 lands, which is why they ship
//! together: before the routing fix nothing constructed a `single_use` grant over
//! a non-exile pool, so neither defect had a carrier.
//!
//! Verbatim Oracle text below is from Scryfall `cards/named` (`exact=`), not from
//! memory.

use engine::ai_support::legal_actions;
use engine::game::combat::AttackTarget;
use engine::game::scenario::{GameRunner, GameScenario, P0, P1};
use engine::parser::oracle::parse_oracle_text;
use engine::types::ability::{CastingPermission, Duration};
use engine::types::actions::GameAction;
use engine::types::game_state::WaitingFor;
use engine::types::identifiers::ObjectId;
use engine::types::mana::ManaCost;
use engine::types::phase::Phase;
use engine::types::zones::Zone;

/// Verbatim Oracle text (Scryfall `cards/named?exact=Locke, Treasure Hunter`).
/// Only the triggered ability is used; the evasion static is irrelevant here and
/// is kept so the parse under test is the card's, not a fragment's.
const LOCKE: &str = "Locke can't be blocked by creatures with greater power.\n\
     Mug — Whenever Locke attacks, each player mills a card. If a land card was \
     milled this way, create a Treasure token. Until end of turn, you may cast a \
     spell from among those cards.";

fn zone_of(runner: &GameRunner, id: ObjectId) -> Zone {
    runner
        .state()
        .objects
        .get(&id)
        .expect("object present")
        .zone
}

fn can_cast(runner: &GameRunner, id: ObjectId) -> bool {
    legal_actions(runner.state())
        .iter()
        .any(|action| matches!(action, GameAction::CastSpell { object_id, .. } if *object_id == id))
}

/// The `single_use` `PlayFromExile` grants recorded on `id`, with the duration
/// each one carries. Reads the permission rather than the legal-action surface,
/// so a test can distinguish "the grant was never installed" from "the grant is
/// installed but something else suppresses the action".
fn single_use_grant_durations(runner: &GameRunner, id: ObjectId) -> Vec<Duration> {
    runner.state().objects[&id]
        .casting_permissions
        .iter()
        .filter_map(|permission| match permission {
            CastingPermission::PlayFromExile {
                duration,
                single_use: true,
                ..
            } => Some(duration.clone()),
            _ => None,
        })
        .collect()
}

/// Two players, Locke attacking, one known card on top of each library.
///
/// Both milled cards are given a zero mana cost so the cap under test is the
/// GRANT's and not the player's mana. `false` is the land flag: CR 305.1 makes a
/// land a special action rather than a cast, and Locke's grant is `mode: Cast`,
/// so a milled land would be uncastable for a reason unrelated to the cap.
struct Milled {
    runner: GameRunner,
    locke: ObjectId,
    mine: ObjectId,
    theirs: ObjectId,
}

fn locke_attacks() -> Milled {
    let mut scenario = GameScenario::new();
    // CR 504.1: start past the draw step. Without this the scenario advances
    // through P0's draw on the way to combat and the card seeded on top of the
    // library is in hand before Locke ever attacks.
    scenario.at_phase(Phase::PreCombatMain);
    let locke = scenario
        .add_creature_from_oracle(P0, "Locke, Treasure Hunter", 3, 3, LOCKE)
        .id();
    let mine = scenario
        .add_spell_to_library_top(P0, "My Milled Card", false)
        .with_mana_cost(ManaCost::zero())
        .id();
    let theirs = scenario
        .add_spell_to_library_top(P1, "Their Milled Card", false)
        .with_mana_cost(ManaCost::zero())
        .id();
    let mut runner = scenario.build();
    runner.advance_to_combat();
    runner
        .declare_attackers(&[(locke, AttackTarget::Player(P1))])
        .expect("Locke must be able to attack");
    runner.advance_until_stack_empty();
    Milled {
        runner,
        locke,
        mine,
        theirs,
    }
}

/// Does this parse still carry the honest `from among` cast-cap refusal anywhere?
///
/// Searched over the serialized tree rather than by walking `AbilityDefinition`
/// by hand: the gap is identified by a NAME that is a single `const` in the
/// parser (`UNREPRESENTABLE_CAST_CAP_GAP`), so a string search over the exported
/// shape cannot drift from it the way a hand-written arm walk can, and the tests
/// here care about "anywhere in the chain" rather than about a position.
fn has_unrepresentable_cast_cap_gap(parsed: &engine::parser::oracle::ParsedAbilities) -> bool {
    serde_json::to_string(parsed)
        .expect("a parsed ability tree serializes")
        .contains("unrepresentable_cast_cap")
}

/// Does this parse install a single-use `PlayFromExile` grant anywhere?
fn has_single_use_grant(parsed: &engine::parser::oracle::ParsedAbilities) -> bool {
    let json = serde_json::to_string(parsed).expect("a parsed ability tree serializes");
    json.contains("\"PlayFromExile\"") && json.contains("\"single_use\":true")
}

/// Walk from the declare-attackers step to the postcombat main phase.
///
/// The scenario driver's `advance_to_phase` passes priority in pairs and stops
/// the moment the engine surfaces something that is not a priority window; CR
/// 509.1 makes declare-blockers a turn-based action rather than a priority
/// window, so the walk has to answer it explicitly. No blocks are declared —
/// this test is about the cast permission, not about combat.
fn advance_past_combat(runner: &mut GameRunner) {
    for _ in 0..40 {
        if runner.state().phase == Phase::PostCombatMain {
            return;
        }
        match runner.state().waiting_for.clone() {
            WaitingFor::DeclareBlockers { .. } => {
                runner
                    .declare_blockers(&[])
                    .expect("the defending player may always decline to block");
            }
            WaitingFor::Priority { .. } => {
                runner
                    .act(GameAction::PassPriority)
                    .expect("passing priority is always legal");
            }
            other => panic!("combat stalled on an unexpected prompt: {other:?}"),
        }
        runner.advance_until_stack_empty();
    }
    panic!("combat did not reach the postcombat main phase");
}

/// CR 603.7 + CR 608.2c: the published set is the MILLED batch and nothing else,
/// and it is published *because* the grant references it.
///
/// Two guarantees in one test, because they share a setup and each is the other's
/// reach guard.
///
/// **(1) The demand link.** Tracked-set publication is DEMAND-DRIVEN:
/// `publish_tracked_set_for_resolution` publishes only when
/// `next_sub_needs_tracked_set` finds a downstream consumer such as
/// `GrantCastingPermission { target: TrackedSet }`. Before this change Locke's
/// cast clause was an `Unimplemented` node that referenced nothing, so the mill
/// published NOTHING (measured: `tracked_object_sets` empty). The routing fix
/// restores the link implicitly rather than by a separate step — which is exactly
/// why it needs pinning. If a refactor ever breaks the link, publication silently
/// stops and the `TrackedSet { id: 0 }` sentinel falls through to
/// `resolve_tracked_set_sentinel`'s fail-open rung, which binds an unrelated
/// earlier set. That failure is invisible at the action surface (cards are still
/// offered — the WRONG cards), so it has to fail loudly here.
///
/// **(2) No Treasure contamination.** Locke creates a token BETWEEN the mill and
/// the cast clause. A token swept into the published set would be offered as a
/// castable member of "those cards", which the card does not say. Could not be
/// tested before the routing fix, because nothing published.
#[test]
fn the_published_set_is_exactly_the_milled_cards_and_excludes_the_treasure() {
    let mut scenario = GameScenario::new();
    scenario.at_phase(Phase::PreCombatMain);
    let locke = scenario
        .add_creature_from_oracle(P0, "Locke, Treasure Hunter", 3, 3, LOCKE)
        .id();
    // A LAND on top, so the conditional "If a land card was milled this way"
    // fires and a Treasure exists to contaminate the set with. Without it this
    // test's contamination half would be vacuous.
    let milled_land = scenario
        .add_spell_to_library_top(P0, "Milled Land", false)
        .as_land()
        .id();
    let theirs = scenario
        .add_spell_to_library_top(P1, "Their Milled Card", false)
        .with_mana_cost(ManaCost::zero())
        .id();
    let mut runner = scenario.build();
    runner.advance_to_combat();
    runner
        .declare_attackers(&[(locke, AttackTarget::Player(P1))])
        .expect("Locke must be able to attack");
    runner.advance_until_stack_empty();

    // Reach guard for the contamination half: a Treasure really was created.
    let treasures: Vec<ObjectId> = runner
        .state()
        .objects
        .iter()
        .filter(|(_, obj)| obj.zone == Zone::Battlefield && obj.name == "Treasure")
        .map(|(id, _)| *id)
        .collect();
    assert!(
        !treasures.is_empty(),
        "reach guard: milling a land must create the Treasure, or the exclusion \
         assertion below has nothing to exclude"
    );

    let published: Vec<Vec<ObjectId>> = runner
        .state()
        .tracked_object_sets
        .values()
        .cloned()
        .collect();
    assert_eq!(
        published.len(),
        1,
        "the demand link must publish exactly one tracked set for this resolution; \
         an EMPTY map means `next_sub_needs_tracked_set` no longer sees the grant \
         and the `TrackedSet {{ id: 0 }}` sentinel will fail open onto an unrelated set"
    );
    let mut members = published.into_iter().next().expect("checked above");
    members.sort();
    let mut expected = vec![milled_land, theirs];
    expected.sort();
    assert_eq!(
        members, expected,
        "CR 608.2c: \"those cards\" is the milled batch — the Treasure created \
         between the mill and the cast clause is not one of them"
    );
    for treasure in treasures {
        assert!(
            single_use_grant_durations(&runner, treasure).is_empty(),
            "the Treasure must not carry the cast grant"
        );
    }
}

/// CR 611.2a: a duration stated by one clause must not promote a LATER capped
/// clause that states none.
///
/// `ParseContext::stated_clause_duration` is the channel that carries a peeled
/// duration to the `from among` mechanism decision, and its failure direction is
/// OPEN: a set/clear lifecycle (rather than save/restore) would leave clause N's
/// duration standing while clause N+1 is lowered, turning a CR 608.2g
/// resolution-window one-shot into a lingering permission the card never printed.
/// That is strictly more permissive than the instruction, and it is invisible to a
/// card-by-card parse delta unless the corpus happens to contain such a pair.
///
/// No printed card exercises this shape today, which is the point: a guard on a
/// currently-unreachable path is the difference between a latent fail-open and a
/// live one.
///
/// DISCRIMINATING: the same second sentence WITH its own leading duration does
/// promote, so this is not "the second clause stopped parsing".
#[test]
fn a_stated_duration_does_not_leak_into_the_next_clause() {
    let leaked = parse_oracle_text(
        "Whenever this creature attacks, until end of turn, creatures you control \
         get +1/+1. Exile the top card of each player's library. You may cast a \
         spell from among those cards.",
        "Leak Probe",
        &[],
        &[],
        &[],
    );
    assert!(
        has_unrepresentable_cast_cap_gap(&leaked),
        "CR 608.2g: the cast clause states no duration of its own, so it must keep \
         its refusal — the duration printed on the FIRST clause is not its"
    );
    assert!(
        !has_single_use_grant(&leaked),
        "a leaked duration must not promote the later clause to a lingering grant"
    );

    // Reach guard: the identical cast sentence, carrying its OWN leading duration,
    // does promote. Without this row the assertions above could pass because the
    // grammar stopped being recognized at all — which is exactly how the first
    // draft of this test went green (its text did not chunk into a chain and the
    // whole line lowered to an unrelated `static_structure` gap).
    let stated = parse_oracle_text(
        "Whenever this creature attacks, until end of turn, creatures you control \
         get +1/+1. Exile the top card of each player's library. Until end of turn, \
         you may cast a spell from among those cards.",
        "Reach Probe",
        &[],
        &[],
        &[],
    );
    assert!(
        has_single_use_grant(&stated),
        "reach guard: the same clause WITH its own stated duration must promote to \
         the single-use grant"
    );
    assert!(
        !has_unrepresentable_cast_cap_gap(&stated),
        "reach guard: the promoted clause must no longer carry the refusal"
    );
}

/// CR 603.7: Locke's grant binds the set THIS resolution published, never an
/// unrelated earlier one.
///
/// `resolve_tracked_set_sentinel` (`game/targeting.rs`) resolves the
/// `TrackedSet { id: 0 }` sentinel through a ladder, and its third rung is
/// `latest_tracked_set_id` — the most recently published set, whatever produced
/// it. `tracked_object_sets` is append-only and never cleared, so that rung is
/// FAIL-OPEN: with nothing published for the current chain it silently binds a
/// stale, unrelated set, the parse stays clean, and the controller is offered
/// arbitrary cards.
///
/// Locke reaches rung 1 (`chain_tracked_set_id`) today because the mill publishes
/// on demand from the grant that references it. That makes the hazard LATENT
/// rather than fixed — and a latent fail-open with no guard is how it comes back.
/// This test makes rung 3 observable: a prior, unrelated tracked set is published
/// first, so if Locke's chain ever stops publishing, the sentinel falls to that
/// set and the assertions below fail with the stale card carrying Locke's grant.
///
/// POSITIVE REACH GUARD: the stale set must actually exist and be DISTINCT from
/// Locke's, or "the stale card has no Locke grant" is vacuously true.
#[test]
fn lockes_grant_binds_this_resolutions_set_not_a_stale_published_one() {
    // Chandra, Hope's Beacon's +1 clause, on an ETB — a different card, whose only
    // job here is to publish a tracked set BEFORE Locke's trigger runs.
    const PRIOR_PUBLISHER: &str = "When this creature enters, exile the top card of your \
         library. Until the end of your next turn, you may cast an instant or sorcery \
         spell from among those exiled cards.";

    let mut scenario = GameScenario::new();
    scenario.at_phase(Phase::PreCombatMain);
    let locke = scenario
        .add_creature_from_oracle(P0, "Locke, Treasure Hunter", 3, 3, LOCKE)
        .id();
    let publisher = scenario
        .add_creature_to_hand_from_oracle(P0, "Prior Publisher", 1, 1, PRIOR_PUBLISHER)
        .with_mana_cost(ManaCost::zero())
        .id();
    // Top of P0's library, added bottom-first: `add_spell_to_library_top` pushes
    // each new card above the previous one, so `stale` ends up on top and the
    // publisher's exile takes it; Locke's mill then takes `mine`.
    let mine = scenario
        .add_spell_to_library_top(P0, "My Milled Card", false)
        .with_mana_cost(ManaCost::zero())
        .id();
    let stale = scenario
        .add_spell_to_library_top(P0, "Stale Set Member", true)
        .with_mana_cost(ManaCost::zero())
        .id();
    let theirs = scenario
        .add_spell_to_library_top(P1, "Their Milled Card", false)
        .with_mana_cost(ManaCost::zero())
        .id();
    let mut runner = scenario.build();

    runner.cast(publisher).commit();
    runner.advance_until_stack_empty();
    let sets_before = runner.state().tracked_object_sets.len();
    assert_eq!(
        sets_before, 1,
        "reach guard: the prior card must publish a tracked set, or there is no \
         stale set for the sentinel to fall onto and this test proves nothing"
    );

    runner.advance_to_combat();
    runner
        .declare_attackers(&[(locke, AttackTarget::Player(P1))])
        .expect("Locke must be able to attack");
    runner.advance_until_stack_empty();

    assert_eq!(
        runner.state().tracked_object_sets.len(),
        2,
        "reach guard: Locke's mill must publish its OWN set alongside the stale one"
    );
    assert_eq!(
        zone_of(&runner, stale),
        Zone::Exile,
        "reach guard: the stale set's member is the exiled card, not a milled one"
    );

    // The load-bearing assertion. Locke's grant is `UntilEndOfTurn`; the prior
    // publisher's is `UntilEndOfNextTurnOf`, so the duration identifies WHICH
    // grant reached the card. Rung 3 would put Locke's grant on the stale card.
    assert!(
        !single_use_grant_durations(&runner, stale).contains(&Duration::UntilEndOfTurn),
        "CR 603.7: Locke's grant must bind the set its own resolution published — \
         finding it on a member of an unrelated earlier set means the sentinel fell \
         through to `latest_tracked_set_id`"
    );
    for (label, id) in [("controller's", mine), ("opponent's", theirs)] {
        assert_eq!(
            single_use_grant_durations(&runner, id),
            vec![Duration::UntilEndOfTurn],
            "{label} milled card must carry Locke's grant"
        );
    }
}

/// CR 611.2a + CR 601.2a: the clause installs a single-use cast grant scoped to
/// the milled batch and bounded by the printed "until end of turn".
///
/// POSITIVE REACH GUARDS, all three required: both cards actually moved
/// Library → Graveyard (the mill ran), Locke is still attacking (the trigger
/// fired from the real combat step rather than from a hand-built ability), and
/// the grant is recorded on a card that is NOT in exile. Without the third, this
/// test would pass unchanged against the pre-fix exile-only machinery.
///
/// DISCRIMINATING: reverting the routing promotion leaves the clause as
/// `Unimplemented { name: "unrepresentable_cast_cap" }`, so no permission is
/// recorded at all and `single_use_grant_durations` returns empty.
#[test]
fn locke_grants_a_single_use_cast_until_end_of_turn() {
    let Milled {
        runner,
        locke,
        mine,
        theirs,
    } = locke_attacks();

    assert_eq!(
        (zone_of(&runner, mine), zone_of(&runner, theirs)),
        (Zone::Graveyard, Zone::Graveyard),
        "reach guard: each player must have milled their top card"
    );
    assert_eq!(
        zone_of(&runner, locke),
        Zone::Battlefield,
        "reach guard: the attack trigger must have fired from a live attacker"
    );

    for (label, id) in [("controller's", mine), ("opponent's", theirs)] {
        assert_eq!(
            single_use_grant_durations(&runner, id),
            vec![Duration::UntilEndOfTurn],
            "{label} milled card must carry the single-use grant bounded by the \
             printed \"Until end of turn\" — a `Duration::Permanent` here means the \
             placeholder was never patched by `apply_duration_to_effect`"
        );
        assert_ne!(
            zone_of(&runner, id),
            Zone::Exile,
            "reach guard ({label}): the grant must be installed on a GRAVEYARD-resident \
             card — this is what the exile-only machinery could not do"
        );
    }
}

/// CR 601.2a: "you may cast **a** spell" is ONE cast across the whole window,
/// shared by every card in the batch.
///
/// This is the assertion the two runtime fixes exist for. With either of them
/// reverted the sibling keeps its grant: without the capture widening the
/// spent-grant ledger is never written (the capture was gated on
/// `source_zone == Zone::Exile` and Locke's pool is the graveyard), and without
/// the zone-blind sweep `consume_single_use_play_from_exile` iterates `state.exile`
/// and never reaches a card sitting in a graveyard.
///
/// POSITIVE REACH GUARDS: the controller's milled card is castable at the action
/// surface before the cast, and BOTH milled cards carry the grant. A bare "the
/// sibling lost its grant" assertion would pass vacuously if the grant had never
/// been installed, which is exactly how this class of test goes green against a
/// broken engine.
///
/// NOT PROVEN HERE, and deliberately: that the opponent's milled card is
/// *offered* at the legal-action surface. It is not, and that is a distinct
/// pre-existing gap this change does not touch —
/// `graveyard_spell_objects_available_to_cast` (`game/casting.rs`) scans only
/// `player_data.graveyard` and then skips `obj.owner != player`, so an
/// owner-scoped graveyard surface hides every non-owner grant, not just Locke's.
/// The exile surface has no such restriction. The grant IS correctly installed on
/// that card (asserted in the sibling test above); only its surfacing is missing.
#[test]
fn locke_authorizes_exactly_one_cast_from_the_milled_batch() {
    let Milled {
        mut runner,
        mine,
        theirs,
        ..
    } = locke_attacks();

    // CR 307.1: the seeded cards are SORCERIES, so the grant is only exercisable
    // at sorcery speed. Walking all the way to the postcombat main phase is also
    // the load-bearing half of "Until end of turn": the permission has to survive
    // the rest of the combat phase to be worth anything.
    advance_past_combat(&mut runner);
    assert_eq!(
        runner.state().phase,
        Phase::PostCombatMain,
        "reach guard: the test must actually reach a sorcery-speed window, or the \
         castability assertions below would be measuring the timing restriction"
    );

    assert!(
        can_cast(&runner, mine),
        "reach guard: the controller's milled card must be castable from the \
         graveyard through the grant before it is spent, or the cap assertion \
         below proves nothing"
    );
    assert_eq!(
        (
            single_use_grant_durations(&runner, mine).len(),
            single_use_grant_durations(&runner, theirs).len()
        ),
        (1, 1),
        "reach guard: both members of the milled batch must hold the grant before \
         it is spent"
    );

    runner.cast(mine).commit();
    runner.advance_until_stack_empty();

    assert!(
        single_use_grant_durations(&runner, theirs).is_empty(),
        "CR 601.2a: the grant printed a cap of ONE, so spending it must strip the \
         permission from every sibling in the batch — including the ones that are \
         not in exile"
    );
    assert!(
        !can_cast(&runner, theirs) && !can_cast(&runner, mine),
        "no member of a spent single-use batch may remain castable"
    );
}
