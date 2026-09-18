//! CR 601.2f: "If multiple cost reductions apply, the player may apply them in
//! any order."
//!
//! Once `CostReductionReach` entered the model, reductions stopped commuting: a
//! `ColoredManaOnly` reduction (the printed "This effect reduces only the
//! amount of colored mana you pay" rider — Morophon, Edgewalker, Ragemonger,
//! the Defiler cycle) and a `SpillsToGeneric` one (CR 118.7b/c/d, the rules
//! default) fight over the same pip, and which one wins decides the total.
//!
//! On `{1}{W}` with a `{W}` colored-only reducer and a `{W}` spillover reducer:
//!   * colored-only FIRST — it takes the white pip, leaving `{1}`; the
//!     spillover unit finds no pip and falls through to generic. Locks `{0}`.
//!   * spillover FIRST — it takes the white pip, leaving `{1}`; the
//!     colored-only unit finds no pip and is discarded. Locks `{1}`.
//!
//! Both are legal CR 601.2f orders, so the engine may not pick for the caster.
//! It asks.
//!
//! The counterweight these tests also pin: prompting on an ORDINARY cast is a
//! bug. A reduction with no colored/colorless component only ever decrements
//! generic mana, which commutes with everything, so it never enters the
//! permutation set — which is why the 492 generic-only `Reduce` statics,
//! Affinity, Undaunted and the one-shot "costs {N} less" reductions are all
//! silent. Every "no prompt" assertion below carries a positive guard proving
//! the reductions really did apply.

use engine::ai_support::candidate_actions_broad;
use engine::game::perf_counters;
use engine::game::scenario::{GameRunner, GameScenario, P0, P1};
use engine::types::ability::{
    AbilityCost, AdditionalCost, AdditionalCostRepeatability, StaticDefinition,
};
use engine::types::actions::GameAction;
use engine::types::casting_costs::{CostReductionEntry, ReductionProvenance};
use engine::types::game_state::{CastPaymentMode, PendingCast, WaitingFor};
use engine::types::identifiers::ObjectId;
use engine::types::mana::{ManaColor, ManaCost, ManaCostShard};
use engine::types::phase::Phase;
use engine::types::player::PlayerId;
use engine::types::statics::{CostModifyMode, CostReductionReach, StaticMode};

fn white() -> ManaCost {
    ManaCost::Cost {
        shards: vec![ManaCostShard::White],
        generic: 0,
    }
}

fn one_white() -> ManaCost {
    ManaCost::Cost {
        shards: vec![ManaCostShard::White],
        generic: 1,
    }
}

fn reducer(amount: ManaCost, reach: CostReductionReach) -> StaticDefinition {
    StaticDefinition::new(StaticMode::ModifyCost {
        mode: CostModifyMode::Reduce,
        amount,
        spell_filter: None,
        dynamic_count: None,
        reach,
    })
}

/// A cost floor in the Trinisphere class (CR 601.2f "effects that directly
/// affect the total cost", applied after every reduction).
fn floor(amount: u32) -> StaticDefinition {
    StaticDefinition::new(StaticMode::ModifyCost {
        mode: CostModifyMode::Minimum,
        amount: ManaCost::generic(amount),
        spell_filter: None,
        dynamic_count: None,
        reach: CostReductionReach::SpillsToGeneric,
    })
}

fn defiler(color: ManaColor, reduction: ManaCost) -> StaticDefinition {
    StaticDefinition::new(StaticMode::DefilerCostReduction {
        color,
        life_cost: 2,
        mana_reduction: reduction,
        reach: CostReductionReach::ColoredManaOnly,
    })
}

struct Board {
    runner: GameRunner,
    spell: ObjectId,
    lands: Vec<ObjectId>,
}

impl Board {
    /// Begin the cast and stop wherever the engine stops.
    fn begin_cast(&mut self) -> Result<(), engine::game::engine::EngineError> {
        let card_id = self.runner.state().objects[&self.spell].card_id;
        self.runner
            .act(GameAction::CastSpell {
                object_id: self.spell,
                card_id,
                targets: vec![],
                payment_mode: CastPaymentMode::Auto,
            })
            .map(|_| ())
    }

    fn tapped_lands(&self) -> usize {
        self.lands
            .iter()
            .filter(|&&id| self.runner.state().objects[&id].tapped)
            .count()
    }

    fn election(&self) -> (&[CostReductionEntry], Vec<ManaCost>, Vec<Vec<usize>>) {
        match &self.runner.state().waiting_for {
            WaitingFor::OrderCostReductions {
                reductions,
                outcomes,
                ..
            } => (
                reductions.as_slice(),
                outcomes.iter().map(|o| o.locked_cost.clone()).collect(),
                outcomes.iter().map(|o| o.order.clone()).collect(),
            ),
            other => panic!("expected an OrderCostReductions prompt, got {other:?}"),
        }
    }
}

/// `reducers` go on separate permanents so each gets its own provenance.
fn board(spell_cost: ManaCost, lands: usize, reducers: Vec<StaticDefinition>) -> Board {
    let mut scenario = GameScenario::new();
    scenario.at_phase(Phase::PreCombatMain);
    let land_ids: Vec<ObjectId> = (0..lands)
        .map(|_| scenario.add_basic_land(P0, ManaColor::White))
        .collect();
    for (index, def) in reducers.into_iter().enumerate() {
        scenario
            .add_creature(P0, &format!("Reducer {index}"), 1, 1)
            .with_static_definition(def);
    }
    let spell = scenario
        .add_creature_to_hand(P0, "Test Spell", 2, 2)
        .with_mana_cost(spell_cost)
        .id();
    Board {
        runner: scenario.build(),
        spell,
        lands: land_ids,
    }
}

/// The headline row: ONE setup, BOTH legal outcomes, driven through the
/// production cast pipeline.
///
/// Revert guard: the two arms assert DIFFERENT totals from the same board. If
/// the engine went back to silently applying colored-only first, the second arm
/// would tap 0 lands instead of 1 and this test reds.
#[test]
fn caster_elects_between_both_legal_cost_reduction_orders() {
    let mut setup = board(
        one_white(),
        3,
        vec![
            reducer(white(), CostReductionReach::ColoredManaOnly),
            reducer(white(), CostReductionReach::SpillsToGeneric),
        ],
    );
    setup.begin_cast().expect("the cast must begin");

    let (reductions, costs, orders) = setup.election();
    assert_eq!(
        reductions.len(),
        2,
        "both shard-bearing reductions must be snapshotted for the election"
    );
    assert_eq!(
        costs,
        vec![ManaCost::generic(0), ManaCost::generic(1)],
        "CR 601.2f: the two legal orders lock {{0}} and {{1}}, cheapest first"
    );
    let cheap_order = orders[0].clone();
    let expensive_order = orders[1].clone();
    assert_ne!(
        cheap_order, expensive_order,
        "the two representatives must be genuinely different orders"
    );

    setup
        .runner
        .act(GameAction::OrderCostReductions {
            order: cheap_order.clone(),
        })
        .expect("the cheapest order must be accepted");
    assert_eq!(
        setup.tapped_lands(),
        0,
        "electing colored-only first locks {{0}} — no land should tap"
    );

    // Same board, the other election.
    let mut setup = board(
        one_white(),
        3,
        vec![
            reducer(white(), CostReductionReach::ColoredManaOnly),
            reducer(white(), CostReductionReach::SpillsToGeneric),
        ],
    );
    setup.begin_cast().expect("the cast must begin");
    setup
        .runner
        .act(GameAction::OrderCostReductions {
            order: expensive_order,
        })
        .expect("the deliberately costlier order must also be accepted");
    assert_eq!(
        setup.tapped_lands(),
        1,
        "electing spillover first locks {{1}} — CR 601.2f lets the caster choose \
         the costlier order, and the engine must honour it"
    );
}

/// The rarity counterweight. Two generic-only reductions both apply, and the
/// caster is never asked, because `max(0, generic - k)` is the same for any
/// interleaving.
///
/// Positive guard: the spell's printed `{3}` drops to `{2}`. Only the
/// `SpillsToGeneric` reducer can touch generic mana — a generic-only
/// `ColoredManaOnly` reduction is inert by design (CR 118.7a: it may only
/// reduce colored mana, and there is none) — so `{2}` is the correct
/// both-reductions-ran number here, not `{1}`. That the total moved at all is
/// what proves the reduction pass ran; do NOT "fix" the assertion to 1.
#[test]
fn generic_only_reductions_never_raise_a_prompt() {
    let mut setup = board(
        ManaCost::generic(3),
        3,
        vec![
            reducer(ManaCost::generic(1), CostReductionReach::ColoredManaOnly),
            reducer(ManaCost::generic(1), CostReductionReach::SpillsToGeneric),
        ],
    );
    setup.begin_cast().expect("the cast must begin");

    assert!(
        !matches!(
            setup.runner.state().waiting_for,
            WaitingFor::OrderCostReductions { .. }
        ),
        "generic-only reductions commute — prompting here would be a bug"
    );
    // CR 118.7a: only the SpillsToGeneric one may touch generic mana, so {3}
    // becomes {2}. That it moved at all proves the reduction pass ran.
    assert_eq!(
        setup.tapped_lands(),
        2,
        "the spillover reduction must still have applied"
    );
}

/// A single shard-bearing reduction has nothing to permute against.
///
/// Positive guard: the reduction demonstrably applied ({1}{W} → {1}).
#[test]
fn a_lone_colored_reduction_never_raises_a_prompt() {
    let mut setup = board(
        one_white(),
        3,
        vec![reducer(white(), CostReductionReach::ColoredManaOnly)],
    );
    setup.begin_cast().expect("the cast must begin");

    assert!(
        !matches!(
            setup.runner.state().waiting_for,
            WaitingFor::OrderCostReductions { .. }
        ),
        "one reduction is not an ordering decision"
    );
    assert_eq!(
        setup.tapped_lands(),
        1,
        "the colored-only reduction must still have cancelled the white pip"
    );
}

/// Two reductions with the SAME reach over the same pip: the order is legal to
/// choose but unobservable, because every permutation locks the same total.
/// Deduping by resulting cost is what keeps this silent.
///
/// Positive guard: the total moved from `{1}{W}` to `{0}`, so both ran.
#[test]
fn same_reach_reductions_lock_one_cost_and_stay_silent() {
    let mut setup = board(
        one_white(),
        3,
        vec![
            reducer(white(), CostReductionReach::SpillsToGeneric),
            reducer(white(), CostReductionReach::SpillsToGeneric),
        ],
    );
    setup.begin_cast().expect("the cast must begin");

    assert!(
        !matches!(
            setup.runner.state().waiting_for,
            WaitingFor::OrderCostReductions { .. }
        ),
        "both orders lock the same total — there is no decision to present"
    );
    assert_eq!(
        setup.tapped_lands(),
        0,
        "one unit takes the pip and the other spills to generic, locking {{0}}"
    );
}

/// Disjoint colours: neither reduction can take the pip the other wants, so the
/// order cannot matter.
///
/// Positive guard: `{1}{W}` becomes `{0}` — the `{W}` unit takes the pip and the
/// `{U}` unit spills onto the `{1}`.
#[test]
fn disjoint_colour_reductions_stay_silent() {
    let mut setup = board(
        one_white(),
        3,
        vec![
            reducer(white(), CostReductionReach::ColoredManaOnly),
            reducer(
                ManaCost::Cost {
                    shards: vec![ManaCostShard::Blue],
                    generic: 0,
                },
                CostReductionReach::SpillsToGeneric,
            ),
        ],
    );
    setup.begin_cast().expect("the cast must begin");

    assert!(
        !matches!(
            setup.runner.state().waiting_for,
            WaitingFor::OrderCostReductions { .. }
        ),
        "reductions that cannot contend for the same pip lock one total"
    );
    assert_eq!(setup.tapped_lands(), 0, "both reductions must have applied");
}

/// A malformed order is rejected and the prompt stays live, so the caster can
/// answer again. The cast is never resolved with an order nobody chose.
#[test]
fn a_malformed_order_is_rejected_and_the_prompt_survives() {
    for bad in [
        vec![0, 0],    // duplicate
        vec![0, 7],    // out of range
        vec![0],       // too short
        vec![0, 1, 0], // too long
    ] {
        let mut setup = board(
            one_white(),
            3,
            vec![
                reducer(white(), CostReductionReach::ColoredManaOnly),
                reducer(white(), CostReductionReach::SpillsToGeneric),
            ],
        );
        setup.begin_cast().expect("the cast must begin");

        let err = setup
            .runner
            .act(GameAction::OrderCostReductions { order: bad.clone() })
            .expect_err("a non-permutation must be refused");
        assert!(
            format!("{err:?}").contains("Cost reduction order"),
            "the refusal must name the offending payload, got {err:?} for {bad:?}"
        );
        assert!(
            matches!(
                setup.runner.state().waiting_for,
                WaitingFor::OrderCostReductions { .. }
            ),
            "the prompt must still be live after refusing {bad:?}"
        );
        assert_eq!(
            setup.tapped_lands(),
            0,
            "no mana may be spent on a refused order"
        );
    }
}

/// CR 601.2e + CR 733: the caster may back out of the whole cast from the
/// ordering prompt, and the game returns to the moment before the cast was
/// proposed.
#[test]
fn the_caster_can_cancel_the_cast_from_the_ordering_prompt() {
    let mut setup = board(
        one_white(),
        3,
        vec![
            reducer(white(), CostReductionReach::ColoredManaOnly),
            reducer(white(), CostReductionReach::SpillsToGeneric),
        ],
    );
    setup.begin_cast().expect("the cast must begin");
    setup
        .runner
        .act(GameAction::CancelCast)
        .expect("cancelling from the ordering prompt must be legal");

    assert!(
        !matches!(
            setup.runner.state().waiting_for,
            WaitingFor::OrderCostReductions { .. }
        ),
        "the prompt must be gone after a cancel"
    );
    assert_eq!(setup.tapped_lands(), 0, "a cancelled cast spends no mana");
    assert!(
        setup.runner.state().players[P0.0 as usize]
            .hand
            .contains(&setup.spell),
        "CR 733: the spell returns to hand"
    );
}

/// The AI's candidate list is exactly the engine's representative set — one
/// action per distinct locked cost, no synthetic permutations bolted on.
#[test]
fn ai_candidates_are_exactly_the_engine_representatives() {
    let mut setup = board(
        one_white(),
        3,
        vec![
            reducer(white(), CostReductionReach::ColoredManaOnly),
            reducer(white(), CostReductionReach::SpillsToGeneric),
        ],
    );
    setup.begin_cast().expect("the cast must begin");
    let (_, _, orders) = setup.election();

    let candidates: Vec<Vec<usize>> = candidate_actions_broad(setup.runner.state())
        .into_iter()
        .filter_map(|candidate| match candidate.action {
            GameAction::OrderCostReductions { order } => Some(order),
            _ => None,
        })
        .collect();

    assert_eq!(
        candidates, orders,
        "every representative must be offered, and nothing else"
    );
}

/// CR 601.2b + CR 601.2f + CR 118.7: an accepted Defiler life payment is a cost
/// reduction like any other, so it is ordered against the board's reductions
/// and then the floor runs LAST.
///
/// Verified arithmetic on a `{W}` white permanent spell with an untapped
/// Trinisphere-class `{3}` floor:
///   * accept  — Defiler removes the `{W}`, leaving `{0}`; the floor lifts it
///     to `{3}` — generic 3, NO shard.
///   * decline — nothing reduces, the floor lifts `{W}` (mana value 1) to
///     `{2}{W}` — generic 2 and the white shard still there.
///
/// The assertion is on the locked `ManaCost` SHAPE, not its mana value: both
/// arms lock a mana value of 3, so counting tapped lands cannot tell "the
/// reduction was ordered before the floor" apart from "the reduction was
/// dropped on the floor" — which is exactly the failure mode a probe
/// `PendingCast` built with the wrong fields produces. Casting in
/// `CastPaymentMode::Manual` parks the cast at its payment step with the locked
/// total still readable on `state.pending_cast`.
///
/// The pre-seam behaviour subtracted the Defiler reduction from the
/// ALREADY-FLOORED cost and produced `{2}`, which is the shape this test also
/// refuses.
#[test]
fn an_accepted_defiler_reduction_is_ordered_before_the_cost_floor() {
    fn locked_cost(pay: bool) -> ManaCost {
        let mut scenario = GameScenario::new();
        scenario.at_phase(Phase::PreCombatMain);
        for _ in 0..5 {
            scenario.add_basic_land(P0, ManaColor::White);
        }
        scenario
            .add_creature(P0, "Defiler of Faith", 4, 4)
            .with_static_definition(defiler(ManaColor::White, white()));
        scenario
            .add_creature(P1, "Trinisphere", 0, 3)
            .with_static_definition(floor(3));
        let spell = scenario
            .add_creature_to_hand(P0, "White Permanent Spell", 2, 2)
            .with_mana_cost(white())
            .id();

        let mut runner = scenario.build();
        let card_id = runner.state().objects[&spell].card_id;
        runner
            .act(GameAction::CastSpell {
                object_id: spell,
                card_id,
                targets: vec![],
                payment_mode: CastPaymentMode::Manual,
            })
            .expect("the cast must begin");
        assert!(
            matches!(
                runner.state().waiting_for,
                WaitingFor::DefilerPayment { .. }
            ),
            "the Defiler offer must be presented before costs are locked in"
        );
        runner
            .act(GameAction::DecideOptionalCost { pay })
            .expect("the Defiler decision must be accepted");
        runner
            .state()
            .pending_cast
            .as_ref()
            .map(|pending| pending.cost.clone())
            .expect("a manual cast parks at its payment step with the locked total")
    }

    assert_eq!(
        locked_cost(true),
        ManaCost::generic(3),
        "CR 601.2f: the Defiler reduction takes the {{W}} to {{0}} and the floor \
         then lifts it to {{3}} — generic only. A locked {{2}} means the old \
         post-floor subtraction is back; a locked {{2}}{{W}} means the accepted \
         reduction was dropped entirely"
    );
    assert_eq!(
        locked_cost(false),
        ManaCost::Cost {
            shards: vec![ManaCostShard::White],
            generic: 2,
        },
        "declining leaves {{W}}, which the same floor lifts to {{2}}{{W}} — the \
         white pip is still there, which is what distinguishes this from the \
         accepted arm's bare {{3}}"
    );
}

/// A Defiler reduction and a board reduction contending for the same pip is a
/// real CR 601.2f election, and the caster's pay/decline answer is what decides
/// whether that election exists at all.
#[test]
fn an_accepted_defiler_reduction_can_create_the_election_itself() {
    fn prompt_after(pay: bool) -> bool {
        let mut scenario = GameScenario::new();
        scenario.at_phase(Phase::PreCombatMain);
        for _ in 0..3 {
            scenario.add_basic_land(P0, ManaColor::White);
        }
        scenario
            .add_creature(P0, "Defiler of Faith", 4, 4)
            .with_static_definition(defiler(ManaColor::White, white()));
        scenario
            .add_creature(P0, "Spillover Reducer", 1, 1)
            .with_static_definition(reducer(white(), CostReductionReach::SpillsToGeneric));
        let spell = scenario
            .add_creature_to_hand(P0, "White Permanent Spell", 2, 2)
            .with_mana_cost(one_white())
            .id();

        let mut runner = scenario.build();
        let card_id = runner.state().objects[&spell].card_id;
        runner
            .act(GameAction::CastSpell {
                object_id: spell,
                card_id,
                targets: vec![],
                payment_mode: CastPaymentMode::Auto,
            })
            .expect("the cast must begin");
        runner
            .act(GameAction::DecideOptionalCost { pay })
            .expect("the Defiler decision must be accepted");
        matches!(
            runner.state().waiting_for,
            WaitingFor::OrderCostReductions { .. }
        )
    }

    assert!(
        prompt_after(true),
        "accepting adds a colored-only {{W}} reduction that contends with the \
         board's spillover {{W}} — that is a real CR 601.2f choice"
    );
    assert!(
        !prompt_after(false),
        "declining leaves one reduction, so there is nothing to order"
    );
}

/// The new wire shapes round-trip, and an old `PendingCast` that predates the
/// two additive fields still parses.
#[test]
fn the_new_wire_shapes_round_trip_and_old_pending_casts_still_parse() {
    let entry = CostReductionEntry {
        amount: white(),
        multiplier: 2,
        reach: CostReductionReach::ColoredManaOnly,
        provenance: ReductionProvenance::Static {
            source: ObjectId(7),
            ordinal: 1,
        },
        display_name: "Morophon, the Boundless".to_string(),
    };
    let encoded = serde_json::to_string(&entry).expect("an entry must serialize");
    let decoded: CostReductionEntry =
        serde_json::from_str(&encoded).expect("an entry must round-trip");
    assert_eq!(decoded, entry);

    for provenance in [
        ReductionProvenance::Defiler,
        ReductionProvenance::Affinity,
        ReductionProvenance::Undaunted,
        ReductionProvenance::PendingOneShot { index: 3 },
    ] {
        let encoded = serde_json::to_string(&provenance).expect("provenance serializes");
        let decoded: ReductionProvenance =
            serde_json::from_str(&encoded).expect("provenance round-trips");
        assert_eq!(decoded, provenance);
    }

    let action = GameAction::OrderCostReductions { order: vec![1, 0] };
    let encoded = serde_json::to_string(&action).expect("the action must serialize");
    let decoded: GameAction = serde_json::from_str(&encoded).expect("the action must round-trip");
    assert_eq!(decoded, action);

    // A v71 `PendingCast` carries neither new field. Take a REAL one from a
    // live cast, serialize it, strip the two additive keys, and parse it back.
    let mut setup = board(
        one_white(),
        3,
        vec![
            reducer(white(), CostReductionReach::ColoredManaOnly),
            reducer(white(), CostReductionReach::SpillsToGeneric),
        ],
    );
    setup.begin_cast().expect("the cast must begin");
    let WaitingFor::OrderCostReductions { pending_cast, .. } = &setup.runner.state().waiting_for
    else {
        panic!("expected the ordering prompt");
    };
    let mut encoded =
        serde_json::to_value(pending_cast.as_ref()).expect("a PendingCast must serialize");
    let object = encoded
        .as_object_mut()
        .expect("a PendingCast serializes as an object");
    object.remove("accepted_cost_reductions");
    object.remove("cost_reduction_election");

    let parsed: PendingCast =
        serde_json::from_value(encoded).expect("a pre-v72 PendingCast must still parse");
    assert!(
        parsed.accepted_cost_reductions.is_empty(),
        "the additive field must default to empty"
    );
    assert!(
        parsed.cost_reduction_election.is_none(),
        "the additive field must default to absent"
    );
}

/// The prompt carries only public information — the spell is already announced
/// and every reduction comes from a face-up battlefield permanent — so it must
/// survive `filter_state_for_player` unchanged for the caster, the opponent,
/// and a spectator alike. Redacting it would strand a reconnecting client.
#[test]
fn the_ordering_prompt_survives_visibility_filtering_for_every_viewer() {
    let mut setup = board(
        one_white(),
        3,
        vec![
            reducer(white(), CostReductionReach::ColoredManaOnly),
            reducer(white(), CostReductionReach::SpillsToGeneric),
        ],
    );
    setup.begin_cast().expect("the cast must begin");
    let (reductions, costs, _) = setup.election();
    let reductions = reductions.to_vec();

    for viewer in [P0, P1, PlayerId(9)] {
        let filtered = engine::game::filter_state_for_viewer(setup.runner.state(), viewer);
        match filtered.waiting_for {
            WaitingFor::OrderCostReductions {
                reductions: seen,
                outcomes,
                ..
            } => {
                assert_eq!(seen, reductions, "viewer {viewer:?} must see the snapshot");
                assert_eq!(
                    outcomes
                        .iter()
                        .map(|o| o.locked_cost.clone())
                        .collect::<Vec<_>>(),
                    costs,
                    "viewer {viewer:?} must see the same outcomes"
                );
            }
            other => panic!("the prompt must survive filtering for {viewer:?}: {other:?}"),
        }
    }
}

/// CR 601.2f: "The total cost is the mana cost ... plus all additional costs
/// and cost increases, and minus all cost reductions."
///
/// The lock seam rebuilds a probe `PendingCast` from `pay_and_push`'s exploded
/// parameters, and every recomputing branch rebuilds the total from
/// `base_cost` — which is the ANNOUNCEMENT-time base and therefore does NOT
/// contain a declared additional mana cost. A probe that drops
/// `declared_mana_additions` silently deletes the kicker from the locked total,
/// which is a rules-illegal underpayment and also makes the `outcomes` the
/// caster and the AI are shown wrong by exactly the kicker.
///
/// Board arithmetic: printed `{1}{W}` + kicker `{2}` = `{3}{W}`.
///   * colored-only first — takes the `{W}`, leaving `{3}`; the spillover unit
///     finds no pip and eats a generic. Locks `{2}`.
///   * spillover first — takes the `{W}`, leaving `{3}`; the colored-only unit
///     finds no pip and is discarded. Locks `{3}`.
///
/// Revert guard: with the kicker dropped from the probe the same board locks
/// `{0}` / `{1}` — the whole election shifts down by the declared `{2}`, and
/// both the outcome assertion and the locked-total assertion red.
#[test]
fn a_declared_kicker_survives_the_cost_reduction_election() {
    let mut scenario = GameScenario::new();
    scenario.at_phase(Phase::PreCombatMain);
    for _ in 0..6 {
        scenario.add_basic_land(P0, ManaColor::White);
    }
    scenario
        .add_creature(P0, "Colored Only Reducer", 1, 1)
        .with_static_definition(reducer(white(), CostReductionReach::ColoredManaOnly));
    scenario
        .add_creature(P0, "Spillover Reducer", 1, 1)
        .with_static_definition(reducer(white(), CostReductionReach::SpillsToGeneric));
    let spell = scenario
        .add_creature_to_hand(P0, "Kicked Spell", 2, 2)
        .with_mana_cost(one_white())
        // CR 702.33a: Kicker {2} — a declared additional MANA cost, so it lands
        // in `PendingCast::declared_mana_additions` rather than in `base_cost`.
        .with_additional_cost(AdditionalCost::Kicker {
            costs: vec![AbilityCost::Mana {
                cost: ManaCost::generic(2),
            }],
            repeatability: AdditionalCostRepeatability::Once,
        })
        .id();

    let mut runner = scenario.build();
    let card_id = runner.state().objects[&spell].card_id;
    runner
        .act(GameAction::CastSpell {
            object_id: spell,
            card_id,
            targets: vec![],
            // Manual parks the cast at its payment step so the LOCKED total is
            // still readable instead of already spent.
            payment_mode: CastPaymentMode::Manual,
        })
        .expect("the cast must begin");
    assert!(
        matches!(
            runner.state().waiting_for,
            WaitingFor::OptionalCostChoice { .. }
        ),
        "the kicker must be offered, got {:?}",
        runner.state().waiting_for
    );
    runner
        .act(GameAction::DecideOptionalCost { pay: true })
        .expect("the kicker must be payable");

    let (costs, orders) = match &runner.state().waiting_for {
        WaitingFor::OrderCostReductions { outcomes, .. } => (
            outcomes
                .iter()
                .map(|o| o.locked_cost.clone())
                .collect::<Vec<_>>(),
            outcomes.iter().map(|o| o.order.clone()).collect::<Vec<_>>(),
        ),
        other => panic!("expected an OrderCostReductions prompt, got {other:?}"),
    };
    assert_eq!(
        costs,
        vec![ManaCost::generic(2), ManaCost::generic(3)],
        "CR 601.2f: the declared kicker is part of the total the election ranges \
         over — {{0}}/{{1}} here would mean the probe dropped it"
    );

    runner
        .act(GameAction::OrderCostReductions {
            order: orders[1].clone(),
        })
        .expect("the costlier order must be accepted");
    let locked = runner
        .state()
        .pending_cast
        .as_ref()
        .map(|pending| pending.cost.clone())
        .expect("a manual cast parks at its payment step with the locked total");
    assert_eq!(
        locked,
        ManaCost::generic(3),
        "the locked total must still contain the declared kicker"
    );
}

/// CR 601.2f: the election is made while `{X}` is still symbolic — this engine
/// runs the lock seam before the Choose-X detour, inverting CR 601.2b's
/// announce-X-first ordering — so the answer has to survive
/// X selection. `apply_post_x_cost_modifiers` re-derives the WHOLE total from
/// `base_cost` once X is concrete; re-deriving it under the pure-projection
/// default silently replaces the caster's elected order with the
/// caster-optimal one, which is precisely the silent engine-side election this
/// whole seam exists to prevent.
///
/// Board arithmetic on printed `{1}{X}{W}` with a `{W}` colored-only and a
/// `{W}` spillover reducer:
///   * while X is symbolic the two legal orders lock `{X}` and `{1}{X}`.
///   * elect the costlier (`{1}{X}`, spillover first) and choose X = 1: the
///     spillover unit takes the white pip off `{2}{W}`, the colored-only unit
///     finds no pip and is discarded. `{2}`.
///
/// Revert guard: re-deriving under `CostFinalizeContext::PREVIEW` applies
/// colored-only first instead, which takes the pip and then spends the
/// spillover unit on a generic — `{1}`, one less than the caster elected.
#[test]
fn an_elected_order_survives_x_selection() {
    let mut scenario = GameScenario::new();
    scenario.at_phase(Phase::PreCombatMain);
    for _ in 0..6 {
        scenario.add_basic_land(P0, ManaColor::White);
    }
    scenario
        .add_creature(P0, "Colored Only Reducer", 1, 1)
        .with_static_definition(reducer(white(), CostReductionReach::ColoredManaOnly));
    scenario
        .add_creature(P0, "Spillover Reducer", 1, 1)
        .with_static_definition(reducer(white(), CostReductionReach::SpillsToGeneric));
    let spell = scenario
        .add_creature_to_hand(P0, "Variable Spell", 2, 2)
        .with_mana_cost(ManaCost::Cost {
            shards: vec![ManaCostShard::X, ManaCostShard::White],
            generic: 1,
        })
        .id();

    let mut runner = scenario.build();
    let card_id = runner.state().objects[&spell].card_id;
    runner
        .act(GameAction::CastSpell {
            object_id: spell,
            card_id,
            targets: vec![],
            payment_mode: CastPaymentMode::Manual,
        })
        .expect("the cast must begin");

    let orders = match &runner.state().waiting_for {
        WaitingFor::OrderCostReductions { outcomes, .. } => {
            assert_eq!(
                outcomes
                    .iter()
                    .map(|o| o.locked_cost.clone())
                    .collect::<Vec<_>>(),
                vec![
                    ManaCost::Cost {
                        shards: vec![ManaCostShard::X],
                        generic: 0,
                    },
                    ManaCost::Cost {
                        shards: vec![ManaCostShard::X],
                        generic: 1,
                    },
                ],
                "The engine defers the CR 601.2b announcement of X until after \
                 the CR 601.2f lock seam, so X is still symbolic here and the \
                 two legal orders differ only in the generic component"
            );
            outcomes.iter().map(|o| o.order.clone()).collect::<Vec<_>>()
        }
        other => panic!("expected an OrderCostReductions prompt, got {other:?}"),
    };

    runner
        .act(GameAction::OrderCostReductions {
            order: orders[1].clone(),
        })
        .expect("the deliberately costlier order must be accepted");
    assert!(
        matches!(runner.state().waiting_for, WaitingFor::ChooseXValue { .. }),
        "Engine ordering, NOT the CR's: CR 601.2b announces the value of a \
         variable cost during announcement, which precedes CR 601.2f's \
         total-cost determination. This engine defers that announcement until \
         after the CR 601.2f lock seam, so the ChooseXValue prompt follows the \
         ordering election rather than preceding it. Got {:?}",
        runner.state().waiting_for
    );

    runner
        .act(GameAction::ChooseX { value: 1 })
        .expect("X = 1 must be choosable");
    let locked = runner
        .state()
        .pending_cast
        .as_ref()
        .map(|pending| pending.cost.clone())
        .expect("a manual cast parks at its payment step with the locked total");
    assert_eq!(
        locked,
        ManaCost::generic(2),
        "CR 601.2f: the caster elected the spillover-first order, and the post-X \
         re-derivation must honour it — {{1}} means the engine re-elected the \
         caster-optimal order behind their back"
    );
}

/// Perf guard for the CR 601.2f analyzer: the permutation search must collect
/// the board ONCE, not once per candidate order.
///
/// `analyze_cost_reduction_order` evaluates every permutation through the real
/// application path. That path reads the board in four counted places — the
/// target-independent and target-dependent modifier collectors, and the two
/// cost-floor channels — so a naive implementation pays `n! * 4` full walks of
/// `game_functioning_statics` inside a single `pay_and_push`. Four shard-bearing
/// reducers (legal outside singleton: four copies of one card) is `4! = 24`
/// orders, i.e. 96 walks that all produce the same collected set.
///
/// `CollectedCastCosts` hoists that collection out of the loop, so the whole
/// cast stays in the low tens of scans regardless of `n!`.
#[test]
fn cost_reduction_order_analysis_collects_the_board_once() {
    let mut setup = board(
        one_white(),
        3,
        vec![
            reducer(white(), CostReductionReach::ColoredManaOnly),
            reducer(white(), CostReductionReach::ColoredManaOnly),
            reducer(white(), CostReductionReach::ColoredManaOnly),
            reducer(white(), CostReductionReach::SpillsToGeneric),
        ],
    );

    perf_counters::reset();
    setup.begin_cast().expect("the cast must begin");
    let scans = perf_counters::snapshot().static_full_scans;

    let (reductions, costs, _) = setup.election();
    assert_eq!(
        reductions.len(),
        4,
        "all four shard-bearing reductions must be snapshotted, giving 4! = 24 \
         candidate orders"
    );
    assert_eq!(
        costs,
        vec![ManaCost::generic(0), ManaCost::generic(1)],
        "CR 601.2f: colored-only first spends the pip and lets the spillover \
         unit eat the generic ({{0}}); spillover first strands all three \
         colored-only units ({{1}})"
    );

    assert!(
        scans < 60,
        "the 24-order search must not re-walk the board per permutation. \
         Measured on this board: 13 scans with the collection hoisted out of \
         the loop, 107 with it inside. Got {scans}"
    );
}

/// Perf guard for the CR 601.2f lock seam on an ORDINARY cast.
///
/// `lock_in_total_cost` runs on every cast, including the simulated ones the AI
/// search drives. Its `cast_can_have_cost_modifiers` fast path only spares
/// boards with ZERO `ModifyCost` statics, so without a second gate every cast
/// made while any generic-only reducer sits on the battlefield pays for the
/// ordering snapshot's two full collector walks — and then throws the result
/// away, because a generic-only reduction is never order-relevant.
///
/// `cast_can_have_order_relevant_reductions` answers "could two SHARD-bearing
/// reductions exist?" by pattern-matching static modes in place, so this board
/// never reaches the collectors at all.
#[test]
fn a_generic_only_board_does_not_pay_for_the_ordering_snapshot() {
    let mut setup = board(
        ManaCost::generic(3),
        3,
        vec![
            reducer(ManaCost::generic(1), CostReductionReach::ColoredManaOnly),
            reducer(ManaCost::generic(1), CostReductionReach::SpillsToGeneric),
        ],
    );

    perf_counters::reset();
    setup.begin_cast().expect("the cast must begin");
    let scans = perf_counters::snapshot().static_full_scans;

    assert!(
        !matches!(
            setup.runner.state().waiting_for,
            WaitingFor::OrderCostReductions { .. }
        ),
        "generic-only reductions commute — prompting here would be a bug"
    );
    // CR 118.7a: only the SpillsToGeneric one may touch generic mana, so {3}
    // becomes {2}. That it moved at all proves the reduction pass ran.
    assert_eq!(
        setup.tapped_lands(),
        2,
        "the spillover reduction must still have applied"
    );

    assert!(
        scans < 11,
        "the lock seam must not snapshot a board that cannot produce an \
         election. Measured on this board: 9 scans with the pre-gate, 11 \
         without it (the snapshot's two collector walks). Got {scans}"
    );
}
