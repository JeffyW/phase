//! CR 702.152 Blitz — em-dash (compound) blitz costs cast from the graveyard.
//!
//! Covers the complete em-dash blitz class, which is exactly two cards:
//!   * Sabin, Master Monk   — "Blitz—{2}{R}{R}, Discard a card."
//!   * Tenacious Underdog   — "Blitz—{2}{B}{B}, Pay 2 life."
//!
//! Both also carry "You may cast this card from your graveyard using its blitz
//! ability.", so the graveyard route is the only route that matters for them.
//!
//! CR 702.152a: "Blitz [cost]" means "You may cast this card by paying [cost]
//! rather than its mana cost". CR 118.9 governs the alternative cost, and
//! CR 601.2h governs paying the non-mana residual (discard / pay life) as part
//! of the total cost.
//!
//! These tests drive the real parser output into a scenario, so they exercise
//! parser -> keyword extraction -> casting-variant selection -> cost payment.

use engine::game::scenario::{GameRunner, GameScenario, P0};
use engine::parser::oracle::parse_oracle_text;
use engine::types::actions::{AlternativeCastDecision, GameAction};
use engine::types::card_type::CoreType;
use engine::types::counter::CounterType;
use engine::types::game_state::{CastPaymentMode, WaitingFor};
use engine::types::identifiers::ObjectId;
use engine::types::keywords::Keyword;
use engine::types::mana::{ManaCost, ManaCostShard, ManaType, ManaUnit};
use engine::types::phase::Phase;
use engine::types::zones::Zone;

const SABIN: &str = "Double strike\nBlitz\u{2014}{2}{R}{R}, Discard a card. (If you cast this spell for its blitz cost, it gains haste and \"When this creature dies, draw a card.\" Sacrifice it at the beginning of the next end step.)\nYou may cast this card from your graveyard using its blitz ability.";

const UNDERDOG: &str = "Blitz\u{2014}{2}{B}{B}, Pay 2 life. (If you cast this spell for its blitz cost, it gains haste and \"When this creature dies, draw a card.\" Sacrifice it at the beginning of the next end step.)\nYou may cast this card from your graveyard using its blitz ability.";

const CALDAIA: &str = "Whenever this creature or another creature you control with mana value 4 or greater dies, create two 1/1 green and white Citizen creature tokens.\nBlitz {2}{G} (If you cast this spell for its blitz cost, it gains haste and \"When this creature dies, draw a card.\" Sacrifice it at the beginning of the next end step.)";

/// Fill the active player's pool with 8 mana of one color — enough to pay any
/// cost in these tests, so an over-charge shows up as a pool delta rather than
/// as an affordability failure.
fn fill_mana(runner: &mut GameRunner, mana: ManaType) {
    let dummy = ObjectId(0);
    let pool = &mut runner.state_mut().players[0].mana_pool;
    for _ in 0..8 {
        pool.add(ManaUnit::new(mana, dummy, false, vec![]));
    }
}

fn blitz_keyword(parsed: &engine::parser::oracle::ParsedAbilities) -> Keyword {
    parsed
        .extracted_keywords
        .iter()
        .find(|k| matches!(k, Keyword::Blitz(_)))
        .unwrap_or_else(|| {
            panic!(
                "blitz keyword must be extracted, got {:?}",
                parsed.extracted_keywords
            )
        })
        .clone()
}

/// CR 702.152a + CR 118.9 + CR 601.2h: Sabin's graveyard blitz charges the
/// blitz cost ({2}{R}{R} = 4 mana), not the printed cost ({4}{R} = 5 mana), and
/// the discard is a mandatory additional cost.
///
/// Before the em-dash branch existed the whole blitz line parsed to
/// `Effect::Unimplemented` and Sabin had no blitz keyword at all, which also
/// made its graveyard-permission static (gated on `HasKeywordKind(Blitz)`) dead.
#[test]
fn sabin_graveyard_blitz_charges_blitz_cost_and_discards() {
    let parsed = parse_oracle_text(
        SABIN,
        "Sabin, Master Monk",
        &[],
        &["Legendary".into(), "Creature".into()],
        &["Human".into(), "Noble".into(), "Monk".into()],
    );
    let kw = blitz_keyword(&parsed);
    let gy_static = parsed
        .statics
        .first()
        .expect("graveyard-cast permission static must parse")
        .clone();

    let mut scenario = GameScenario::new();
    scenario.at_phase(Phase::PreCombatMain);
    let sabin = scenario
        .add_creature_to_graveyard(P0, "Sabin, Master Monk", 4, 3)
        .with_static_definition(gy_static)
        // Printed cost {4}{R} = 5 mana; blitz cost is 4. The two differ, so the
        // pool delta discriminates which cost was actually charged.
        .with_mana_cost(ManaCost::Cost {
            generic: 4,
            shards: vec![ManaCostShard::Red],
        })
        .with_keyword(kw)
        .id();
    scenario.add_card_to_hand(P0, "Filler Card");
    let mut runner = scenario.build();
    fill_mana(&mut runner, ManaType::Red);

    let card_id = runner.state().objects[&sabin].card_id;
    let waiting = runner
        .act(GameAction::CastSpell {
            object_id: sabin,
            card_id,
            targets: vec![],
            payment_mode: CastPaymentMode::Auto,
        })
        .expect("graveyard blitz cast must be legal");

    // CR 118.9a: only one alternative cost applies, and the permission reads
    // "using its blitz ability" — so the gate must NOT also offer a printed-cost
    // graveyard cast. Reaching a bare cost payment (not a variant choice) is the
    // positive reach guard that the blitz variant was selected outright.
    assert!(
        !matches!(waiting.waiting_for, WaitingFor::CastingVariantChoice { .. }),
        "a \"using its blitz ability\" permission must not offer a printed-cost \
         graveyard cast alongside blitz; got {:?}",
        waiting.waiting_for
    );
    assert!(
        matches!(waiting.waiting_for, WaitingFor::PayCost { .. }),
        "expected the blitz discard cost prompt, got {:?}",
        waiting.waiting_for
    );

    // CR 601.2h: the discard is part of the total cost, so it cannot be declined.
    assert!(
        runner
            .act(GameAction::SelectCards { cards: vec![] })
            .is_err(),
        "blitz's discard is an additional cost and must be mandatory"
    );

    let filler = runner.state().players[0].hand[0];
    runner
        .act(GameAction::SelectCards {
            cards: vec![filler],
        })
        .expect("paying the blitz discard must succeed");

    assert_eq!(
        runner.state().players[0].mana_pool.total(),
        4,
        "blitz cost {{2}}{{R}}{{R}} = 4 mana must be charged, not the printed {{4}}{{R}} = 5"
    );
    assert_eq!(
        runner.state().players[0].hand.len(),
        0,
        "the discarded card must leave hand"
    );
    assert_eq!(
        runner.state().stack.len(),
        1,
        "Sabin must be on the stack after a successful blitz cast"
    );
}

/// CR 702.152a: Tenacious Underdog — the other member of the em-dash blitz
/// class, with a pay-life residual instead of a discard. This card carried NO
/// `Unimplemented` marker before the fix: it silently charged its printed cost.
///
/// Its printed cost ({1}{B} = 2) is CHEAPER than its blitz cost ({2}{B}{B} = 4),
/// so a pool delta of 4 cannot be produced by accidentally charging the printed
/// cost — the direction of the difference makes this assertion discriminating.
#[test]
fn underdog_graveyard_blitz_charges_blitz_cost_and_pays_life() {
    let parsed = parse_oracle_text(
        UNDERDOG,
        "Tenacious Underdog",
        &[],
        &["Creature".into()],
        &["Human".into(), "Warrior".into()],
    );
    let kw = blitz_keyword(&parsed);
    let gy_static = parsed
        .statics
        .first()
        .expect("graveyard-cast permission static must parse")
        .clone();

    let mut scenario = GameScenario::new();
    scenario.at_phase(Phase::PreCombatMain);
    let dog = scenario
        .add_creature_to_graveyard(P0, "Tenacious Underdog", 3, 2)
        .with_static_definition(gy_static)
        .with_mana_cost(ManaCost::Cost {
            generic: 1,
            shards: vec![ManaCostShard::Black],
        })
        .with_keyword(kw)
        .id();
    let mut runner = scenario.build();
    fill_mana(&mut runner, ManaType::Black);
    let life_before = runner.state().players[0].life;

    let card_id = runner.state().objects[&dog].card_id;
    let waiting = runner
        .act(GameAction::CastSpell {
            object_id: dog,
            card_id,
            targets: vec![],
            payment_mode: CastPaymentMode::Auto,
        })
        .expect("graveyard blitz cast must be legal");

    assert!(
        !matches!(waiting.waiting_for, WaitingFor::CastingVariantChoice { .. }),
        "a \"using its blitz ability\" permission must not offer a printed-cost \
         graveyard cast alongside blitz; got {:?}",
        waiting.waiting_for
    );

    assert_eq!(
        runner.state().players[0].mana_pool.total(),
        4,
        "blitz cost {{2}}{{B}}{{B}} = 4 mana must be charged, not the printed {{1}}{{B}} = 2"
    );
    assert_eq!(
        runner.state().players[0].life,
        life_before - 2,
        "CR 601.2h: blitz's \"Pay 2 life\" residual must actually be paid"
    );
    assert_eq!(
        runner.state().stack.len(),
        1,
        "Tenacious Underdog must be on the stack after a successful blitz cast"
    );
}

/// CR 118.9b: alternative costs are OPTIONAL. When a separate, unconstrained
/// permission authorizes casting from the graveyard (Advanced Floral
/// Invocations' "You may play lands and cast creature spells from your
/// graveyard.", and the Muldrotha / Lurrus class generally), the printed-cost
/// graveyard cast IS on offer — so a blitz creature there must present the
/// CHOICE rather than being force-routed into blitz.
///
/// This is distinct from the two class tests above, where the only permission is
/// the card's own "using its blitz ability" rider and blitz is the sole legal
/// cast.
#[test]
fn unconstrained_graveyard_permission_still_offers_printed_cost_choice() {
    const INVOCATIONS: &str = "You may play lands and cast creature spells from your graveyard.";

    let parsed = parse_oracle_text(
        SABIN,
        "Sabin, Master Monk",
        &[],
        &["Legendary".into(), "Creature".into()],
        &["Human".into(), "Noble".into(), "Monk".into()],
    );
    let kw = blitz_keyword(&parsed);
    let own_rider = parsed
        .statics
        .first()
        .expect("graveyard-cast permission static must parse")
        .clone();

    let enabler = parse_oracle_text(
        INVOCATIONS,
        "Advanced Floral Invocations",
        &[],
        &["Enchantment".into()],
        &[],
    );
    let unconstrained = enabler
        .statics
        .iter()
        .find(|s| {
            format!("{s:?}").contains("GraveyardCastPermission")
                && !format!("{s:?}").contains("HasKeywordKind")
        })
        .expect("an unconstrained GraveyardCastPermission must parse")
        .clone();

    let mut scenario = GameScenario::new();
    scenario.at_phase(Phase::PreCombatMain);
    scenario
        .add_enchantment_from_oracle(P0, "Advanced Floral Invocations", INVOCATIONS)
        .with_static_definition(unconstrained);
    let sabin = scenario
        .add_creature_to_graveyard(P0, "Sabin, Master Monk", 4, 3)
        .with_static_definition(own_rider)
        .with_mana_cost(ManaCost::Cost {
            generic: 4,
            shards: vec![ManaCostShard::Red],
        })
        .with_keyword(kw)
        .id();
    scenario.add_card_to_hand(P0, "Filler Card");
    let mut runner = scenario.build();
    fill_mana(&mut runner, ManaType::Red);

    let card_id = runner.state().objects[&sabin].card_id;
    let waiting = runner
        .act(GameAction::CastSpell {
            object_id: sabin,
            card_id,
            targets: vec![],
            payment_mode: CastPaymentMode::Auto,
        })
        .expect("graveyard cast must be legal under an unconstrained permission");

    assert!(
        matches!(
            waiting.waiting_for,
            WaitingFor::AlternativeCastChoice {
                keyword: engine::types::game_state::AlternativeCastKeyword::Blitz,
                ..
            }
        ),
        "with a printed-cost graveyard cast also legal, blitz must be OFFERED, \
         not forced; got {:?}",
        waiting.waiting_for
    );
}

/// Anti-widening control: Caldaia Guardian has space-form `Blitz {2}{G}` and NO
/// graveyard-cast permission. Blitz alone must never make a card castable from
/// the graveyard — only the separate permission static does that.
///
/// This is what stops the new `blitz_castable_zone` gate widening across the 14
/// space-form blitz cards. Note the two class tests above supply their own
/// permission static, so this control is not measuring assertion order: it is
/// the same runtime path with the permission removed.
#[test]
fn space_form_blitz_alone_does_not_allow_graveyard_cast() {
    let parsed = parse_oracle_text(
        CALDAIA,
        "Caldaia Guardian",
        &["Blitz".into()],
        &["Creature".into()],
        &["Human".into(), "Soldier".into()],
    );
    let kw = blitz_keyword(&parsed);
    assert!(
        parsed.statics.is_empty(),
        "Caldaia Guardian must have no graveyard-cast permission; got {:?}",
        parsed.statics
    );

    let mut scenario = GameScenario::new();
    scenario.at_phase(Phase::PreCombatMain);
    let guardian = scenario
        .add_creature_to_graveyard(P0, "Caldaia Guardian", 4, 3)
        .with_mana_cost(ManaCost::Cost {
            generic: 3,
            shards: vec![ManaCostShard::Green],
        })
        .with_keyword(kw)
        .id();
    let mut runner = scenario.build();
    fill_mana(&mut runner, ManaType::Green);

    let card_id = runner.state().objects[&guardian].card_id;
    let res = runner.act(GameAction::CastSpell {
        object_id: guardian,
        card_id,
        targets: vec![],
        payment_mode: CastPaymentMode::Auto,
    });
    assert!(
        res.is_err(),
        "blitz without a graveyard-cast permission must not be castable from the \
         graveyard, but the cast was accepted: {:?}",
        res.map(|w| w.waiting_for)
    );
    assert_eq!(
        runner.state().stack.len(),
        0,
        "no spell may reach the stack from this illegal cast"
    );
}

const MULDROTHA: &str = "During each of your turns, you may play a land and cast a permanent spell of each permanent type from your graveyard. (If a card has multiple permanent types, choose one as you play it.)";

const LEONARDO: &str = "Sneak {2}{W}{W}\nDouble strike\nDuring your turn, you may cast creature spells with power or toughness 1 or less from your graveyard. If you cast a spell this way, that creature enters with a finality counter on it. (If a creature with a finality counter on it would die, exile it instead.)";

/// Put a permanent on P0's battlefield carrying every static its Oracle text
/// parses to, so the graveyard permission under test is the parser's own.
fn add_permission_source(
    scenario: &mut GameScenario,
    name: &str,
    oracle: &str,
    subtypes: &[&str],
) -> ObjectId {
    let parsed = parse_oracle_text(
        oracle,
        name,
        &[],
        &["Legendary".into(), "Creature".into()],
        &subtypes
            .iter()
            .map(|s| (*s).to_string())
            .collect::<Vec<_>>(),
    );
    assert!(
        parsed
            .statics
            .iter()
            .any(|s| format!("{s:?}").contains("GraveyardCastPermission")),
        "{name} must parse to a GraveyardCastPermission, got {:?}",
        parsed.statics
    );
    let mut source = scenario.add_creature(P0, name, 4, 4);
    for s in parsed.statics {
        source.with_static_definition(s);
    }
    source.id()
}

fn caldaia_blitz() -> Keyword {
    blitz_keyword(&parse_oracle_text(
        CALDAIA,
        "Caldaia Guardian",
        &["Blitz".into()],
        &["Creature".into()],
        &["Human".into(), "Soldier".into()],
    ))
}

fn cast_from_graveyard(runner: &mut GameRunner, id: ObjectId) -> Result<WaitingFor, String> {
    let card_id = runner.state().objects[&id].card_id;
    runner
        .act(GameAction::CastSpell {
            object_id: id,
            card_id,
            targets: vec![],
            payment_mode: CastPaymentMode::Auto,
        })
        .map(|r| r.waiting_for)
        .map_err(|e| format!("{e:?}"))
}

/// CR 601.2a + CR 118.9a: a card cast from the graveyard for its own
/// alternative cost is still cast under the permission that let it leave the
/// graveyard. Muldrotha allows one permanent spell of each permanent type per
/// turn, so a creature blitzed from the graveyard spends Muldrotha's creature
/// slot, and a second creature can't follow it that turn.
///
/// Before the fix, choosing blitz replaced the `GraveyardPermission` casting
/// variant, so finalization never spent the slot and Muldrotha allowed one
/// graveyard creature after another.
#[test]
fn blitz_from_graveyard_under_muldrotha_spends_its_creature_slot() {
    let mut scenario = GameScenario::new();
    scenario.at_phase(Phase::PreCombatMain);
    let muldrotha = add_permission_source(
        &mut scenario,
        "Muldrotha, the Gravetide",
        MULDROTHA,
        &["Elemental", "Avatar"],
    );
    let guardian = scenario
        .add_creature_to_graveyard(P0, "Caldaia Guardian", 4, 3)
        .with_mana_cost(ManaCost::Cost {
            generic: 3,
            shards: vec![ManaCostShard::Green],
        })
        .with_keyword(caldaia_blitz())
        .id();
    let bears = scenario
        .add_creature_to_graveyard(P0, "Grizzly Bears", 2, 2)
        .with_mana_cost(ManaCost::Cost {
            generic: 1,
            shards: vec![ManaCostShard::Green],
        })
        .id();
    let mut runner = scenario.build();
    fill_mana(&mut runner, ManaType::Green);

    // Muldrotha is unconstrained, so the printed cost is also on offer and
    // blitz must be chosen (CR 118.9b).
    let waiting = cast_from_graveyard(&mut runner, guardian).expect("graveyard cast must be legal");
    assert!(
        matches!(
            waiting,
            WaitingFor::AlternativeCastChoice {
                keyword: engine::types::game_state::AlternativeCastKeyword::Blitz,
                ..
            }
        ),
        "expected the blitz choice, got {waiting:?}"
    );
    runner
        .act(GameAction::ChooseAlternativeCast {
            choice: AlternativeCastDecision::Alternative,
        })
        .expect("choosing blitz must complete the cast");

    // Positive reach guard: the blitz cast completed and charged blitz's
    // {2}{G} = 3 mana rather than the printed {3}{G} = 4.
    assert_eq!(
        runner.state().stack.len(),
        1,
        "Caldaia must be on the stack"
    );
    assert_eq!(
        runner.state().players[0].mana_pool.total(),
        5,
        "blitz {{2}}{{G}} = 3 of the 8 mana must be spent, not the printed 4"
    );

    assert!(
        runner
            .state()
            .graveyard_cast_permissions_used_per_type
            .contains(&(muldrotha, CoreType::Creature)),
        "the blitz cast must spend Muldrotha's creature slot, used: {:?}",
        runner.state().graveyard_cast_permissions_used_per_type
    );

    runner.resolve_top();
    assert_eq!(
        runner.state().objects[&guardian].zone,
        Zone::Battlefield,
        "Caldaia must resolve, so the stack is empty for the next sorcery-speed cast"
    );
    assert!(runner.state().stack.is_empty());

    let second = cast_from_graveyard(&mut runner, bears);
    assert!(
        second.is_err(),
        "Muldrotha's creature slot is spent, so a second graveyard creature \
         must be refused this turn, but the cast was accepted: {second:?}"
    );
    assert!(
        runner.state().stack.is_empty(),
        "the refused cast must not reach the stack"
    );
}

/// CR 601.2a: when an unlimited permission also admits the blitz cast, the
/// bounded one is not spent. Sabin's own rider ("You may cast this card from
/// your graveyard using its blitz ability.") needs no slot, so blitzing Sabin
/// beside Muldrotha leaves Muldrotha's creature slot for another creature.
#[test]
fn blitz_prefers_the_cards_own_rider_over_a_bounded_graveyard_permission() {
    let parsed = parse_oracle_text(
        SABIN,
        "Sabin, Master Monk",
        &[],
        &["Legendary".into(), "Creature".into()],
        &["Human".into(), "Noble".into(), "Monk".into()],
    );
    let own_rider = parsed
        .statics
        .first()
        .expect("graveyard-cast permission static must parse")
        .clone();

    let mut scenario = GameScenario::new();
    scenario.at_phase(Phase::PreCombatMain);
    let muldrotha = add_permission_source(
        &mut scenario,
        "Muldrotha, the Gravetide",
        MULDROTHA,
        &["Elemental", "Avatar"],
    );
    let sabin = scenario
        .add_creature_to_graveyard(P0, "Sabin, Master Monk", 4, 3)
        .with_static_definition(own_rider)
        .with_mana_cost(ManaCost::Cost {
            generic: 4,
            shards: vec![ManaCostShard::Red],
        })
        .with_keyword(blitz_keyword(&parsed))
        .id();
    let bears = scenario
        .add_creature_to_graveyard(P0, "Grizzly Bears", 2, 2)
        .with_mana_cost(ManaCost::Cost {
            generic: 1,
            shards: vec![ManaCostShard::Red],
        })
        .id();
    scenario.add_card_to_hand(P0, "Filler Card");
    let mut runner = scenario.build();
    fill_mana(&mut runner, ManaType::Red);

    cast_from_graveyard(&mut runner, sabin).expect("graveyard cast must be legal");
    runner
        .act(GameAction::ChooseAlternativeCast {
            choice: AlternativeCastDecision::Alternative,
        })
        .expect("choosing blitz must be legal");
    let filler = runner.state().players[0].hand[0];
    runner
        .act(GameAction::SelectCards {
            cards: vec![filler],
        })
        .expect("paying the blitz discard must complete the cast");

    // Positive reach guard: the blitz cast completed for {2}{R}{R} = 4.
    assert_eq!(runner.state().stack.len(), 1, "Sabin must be on the stack");
    assert_eq!(runner.state().players[0].mana_pool.total(), 4);

    assert!(
        !runner
            .state()
            .graveyard_cast_permissions_used_per_type
            .contains(&(muldrotha, CoreType::Creature)),
        "Sabin's own unlimited rider authorizes this cast, so Muldrotha's \
         creature slot must stay unspent, used: {:?}",
        runner.state().graveyard_cast_permissions_used_per_type
    );

    runner.resolve_top();
    assert!(runner.state().stack.is_empty());
    cast_from_graveyard(&mut runner, bears)
        .expect("Muldrotha's creature slot is still free, so this cast must be legal");
}

/// CR 601.2a + CR 122.1: a permission's "if you cast a spell this way, that
/// creature enters with a counter on it" rider applies to a blitz cast it
/// admits, because blitz changes the cost, not the permission. Leonardo admits
/// creature spells with power or toughness 1 or less from the graveyard, so the
/// fixture is a 1/1 blitz creature.
#[test]
fn blitz_from_graveyard_keeps_the_permissions_enters_with_counter_rider() {
    let mut scenario = GameScenario::new();
    scenario.at_phase(Phase::PreCombatMain);
    add_permission_source(
        &mut scenario,
        "Leonardo, Sewer Samurai",
        LEONARDO,
        &["Mutant", "Ninja", "Turtle", "Samurai"],
    );
    let blitzer = scenario
        .add_creature_to_graveyard(P0, "Blitz Test Creature", 1, 1)
        .with_mana_cost(ManaCost::Cost {
            generic: 3,
            shards: vec![ManaCostShard::Green],
        })
        .with_keyword(caldaia_blitz())
        .id();
    let mut runner = scenario.build();
    fill_mana(&mut runner, ManaType::Green);

    let waiting = cast_from_graveyard(&mut runner, blitzer).expect("graveyard cast must be legal");
    assert!(
        matches!(waiting, WaitingFor::AlternativeCastChoice { .. }),
        "expected the blitz choice, got {waiting:?}"
    );
    runner
        .act(GameAction::ChooseAlternativeCast {
            choice: AlternativeCastDecision::Alternative,
        })
        .expect("choosing blitz must complete the cast");
    // Positive reach guard: blitz's {2}{G} = 3 was charged, not the printed 4.
    assert_eq!(runner.state().players[0].mana_pool.total(), 5);

    runner.resolve_top();
    let entered = &runner.state().objects[&blitzer];
    assert_eq!(entered.zone, Zone::Battlefield);
    assert_eq!(
        entered.counters.get(&CounterType::Finality).copied(),
        Some(1),
        "Leonardo's finality-counter rider must apply to a blitz cast it admits, \
         counters: {:?}",
        entered.counters
    );
}
