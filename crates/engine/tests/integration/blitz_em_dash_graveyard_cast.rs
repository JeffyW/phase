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
use engine::types::mana::{ManaColor, ManaCost, ManaCostShard, ManaType, ManaUnit};
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

const RIVETEERS_DECOY: &str = "This creature must be blocked if able.\nBlitz {3}{G} (If you cast this spell for its blitz cost, it gains haste and \"When this creature dies, draw a card.\" Sacrifice it at the beginning of the next end step.)";

const BOON_SATYR: &str = "Flash\nBestow {3}{G}{G} (If you cast this card for its bestow cost, it's an Aura spell with enchant creature. It becomes a creature again if it's not attached.)\nEnchanted creature gets +4/+2.";

/// CR 601.2a + CR 110.4 + CR 702.103b: the fix applies to every alternative
/// cost that is the card's own, not only Blitz. Bestow is the other one that can
/// be cast from the graveyard.
///
/// A bestowed spell is an Aura, not a creature (CR 702.103b), so under Muldrotha
/// it is cast as an enchantment spell and spends the ENCHANTMENT slot, leaving
/// the creature slot free (Muldrotha ruling, 2020-11-10: "you can cast a card
/// with bestow as an enchantment spell"). Before the fix it spent neither.
#[test]
fn bestow_from_graveyard_under_muldrotha_spends_its_enchantment_slot() {
    let mut scenario = GameScenario::new();
    scenario.at_phase(Phase::PreCombatMain);
    let muldrotha = add_permission_source(
        &mut scenario,
        "Muldrotha, the Gravetide",
        MULDROTHA,
        &["Elemental", "Avatar"],
    );
    let mut builder = scenario.add_creature_to_graveyard(P0, "Boon Satyr", 4, 2);
    builder.with_mana_cost(ManaCost::Cost {
        generic: 1,
        shards: vec![ManaCostShard::Green, ManaCostShard::Green],
    });
    builder.with_subtypes(vec!["Satyr"]);
    builder.from_oracle_text_with_keywords(&["Flash", "Bestow"], BOON_SATYR);
    let satyr = builder.id();
    let mut runner = scenario.build();
    // Boon Satyr is an Enchantment Creature. The graveyard builder seeds only
    // Creature, so add Enchantment to both the current and base type lines.
    {
        let obj = runner.state_mut().objects.get_mut(&satyr).unwrap();
        for types in [
            &mut obj.card_types.core_types,
            &mut obj.base_card_types.core_types,
        ] {
            if !types.contains(&CoreType::Enchantment) {
                types.push(CoreType::Enchantment);
            }
        }
    }
    fill_mana(&mut runner, ManaType::Green);

    cast_from_graveyard(&mut runner, satyr).expect("graveyard bestow cast must be legal");
    if let WaitingFor::TargetSelection { .. } = runner.state().waiting_for {
        runner
            .choose_first_legal_target()
            .expect("Muldrotha is a legal creature to enchant");
    }

    // Positive reach guard: the bestow cast completed, charging bestow's
    // {3}{G}{G} = 5 rather than the printed {1}{G}{G} = 3.
    assert_eq!(
        runner.state().stack.len(),
        1,
        "Boon Satyr must be on the stack"
    );
    assert_eq!(runner.state().players[0].mana_pool.total(), 3);

    let used = &runner.state().graveyard_cast_permissions_used_per_type;
    assert!(
        used.contains(&(muldrotha, CoreType::Enchantment)),
        "a bestowed spell is an enchantment spell, so it must spend Muldrotha's \
         enchantment slot, used: {used:?}"
    );
    assert!(
        !used.contains(&(muldrotha, CoreType::Creature)),
        "a bestowed spell is not a creature spell, so the creature slot must stay \
         free, used: {used:?}"
    );
}

/// CR 601.2a: when several permissions admit a graveyard cast, the player
/// announces which one they use (Muldrotha, 2020-11-10 ruling). The engine
/// picks only when the choice is strictly dominant, so an unlimited permission
/// that brings a rider is NOT preferred over a bounded one.
///
/// Leonardo is unlimited but gives the creature a finality counter, so blitz's
/// end-step sacrifice would exile Riveteers Decoy instead of returning it to the
/// graveyard. Spending Muldrotha's slot instead is a real alternative, so the
/// engine keeps the same source-order first match a printed-cost cast gets.
/// Muldrotha is added first, so that first match is Muldrotha.
#[test]
fn blitz_does_not_prefer_an_unlimited_permission_that_brings_a_rider() {
    let decoy_blitz = blitz_keyword(&parse_oracle_text(
        RIVETEERS_DECOY,
        "Riveteers Decoy",
        &["Blitz".into()],
        &["Creature".into()],
        &["Human".into(), "Warrior".into()],
    ));

    let mut scenario = GameScenario::new();
    scenario.at_phase(Phase::PreCombatMain);
    let muldrotha = add_permission_source(
        &mut scenario,
        "Muldrotha, the Gravetide",
        MULDROTHA,
        &["Elemental", "Avatar"],
    );
    add_permission_source(
        &mut scenario,
        "Leonardo, Sewer Samurai",
        LEONARDO,
        &["Mutant", "Ninja", "Turtle", "Samurai"],
    );
    let decoy = scenario
        .add_creature_to_graveyard(P0, "Riveteers Decoy", 3, 1)
        .with_mana_cost(ManaCost::Cost {
            generic: 1,
            shards: vec![ManaCostShard::Green],
        })
        .with_keyword(decoy_blitz)
        .id();
    let mut runner = scenario.build();
    fill_mana(&mut runner, ManaType::Green);

    cast_from_graveyard(&mut runner, decoy).expect("graveyard cast must be legal");
    runner
        .act(GameAction::ChooseAlternativeCast {
            choice: AlternativeCastDecision::Alternative,
        })
        .expect("choosing blitz must complete the cast");
    // Positive reach guard: blitz's {3}{G} = 4 was charged, not the printed 2.
    assert_eq!(runner.state().players[0].mana_pool.total(), 4);

    assert!(
        runner
            .state()
            .graveyard_cast_permissions_used_per_type
            .contains(&(muldrotha, CoreType::Creature)),
        "with no strictly dominant permission, the source-order first match \
         (Muldrotha) authorizes the cast and spends its slot, used: {:?}",
        runner.state().graveyard_cast_permissions_used_per_type
    );

    runner.resolve_top();
    let entered = &runner.state().objects[&decoy];
    assert_eq!(entered.zone, Zone::Battlefield);
    assert_eq!(
        entered.counters.get(&CounterType::Finality).copied(),
        None,
        "Leonardo's permission was not the one used, so its finality rider must \
         not apply, counters: {:?}",
        entered.counters
    );
}

const EXPLORATION_BROODSHIP: &str = "Station (Tap another creature you control: Put charge counters equal to its power on this Spacecraft. Station only as a sorcery. It's an artifact creature at 8+.)\n3+ | You may play an additional land on each of your turns.\n8+ | Flying\nOnce during each of your turns, you may cast a permanent spell from your graveyard by sacrificing a land in addition to paying its other costs.";

const ENCROACHING_MYCOSYNTH: &str = "Nonland permanents you control are artifacts in addition to their other types. The same is true for permanent spells you control and nonland permanent cards you own that aren't on the battlefield.";

/// Answer every `PayCost` prompt with the first card it offers, until priority
/// returns or a prompt this helper does not handle comes up.
fn pay_offered_costs(runner: &mut GameRunner) {
    for _ in 0..4 {
        let choice = match &runner.state().waiting_for {
            WaitingFor::PayCost { choices, .. } => choices.first().copied(),
            _ => return,
        };
        runner
            .act(GameAction::SelectCards {
                cards: choice.into_iter().collect(),
            })
            .expect("paying an offered cost must be legal");
    }
}

/// CR 601.2a + CR 601.2f: the permission that sets a graveyard cast's cost is
/// the one it commits to. Sabin's own blitz rider needs no slot and has no
/// extra cost, so it is elected over Exploration Broodship, and Broodship's
/// "by sacrificing a land" rider must NOT be charged. Broodship is on the
/// battlefield, so it is scanned before Sabin's own rider.
///
/// Before the election was shared, payment read Broodship's rider (source-order
/// first match) while finalization spent Sabin's own rider: the land was
/// sacrificed and Broodship's slot was left unspent.
#[test]
fn blitz_charges_only_the_elected_permissions_extra_cost() {
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
    let broodship = scenario
        .add_artifact_from_oracle(P0, "Exploration Broodship", EXPLORATION_BROODSHIP)
        .id();
    let land = scenario.add_basic_land(P0, ManaColor::Red);
    let sabin = scenario
        .add_creature_to_graveyard(P0, "Sabin, Master Monk", 4, 3)
        .with_static_definition(own_rider)
        .with_mana_cost(ManaCost::Cost {
            generic: 4,
            shards: vec![ManaCostShard::Red],
        })
        .with_keyword(blitz_keyword(&parsed))
        .id();
    let filler = scenario.add_card_to_hand(P0, "Filler Card");
    let mut runner = scenario.build();
    fill_mana(&mut runner, ManaType::Red);

    cast_from_graveyard(&mut runner, sabin).expect("graveyard cast must be legal");
    runner
        .act(GameAction::ChooseAlternativeCast {
            choice: AlternativeCastDecision::Alternative,
        })
        .expect("choosing blitz must be legal");
    pay_offered_costs(&mut runner);

    // Positive reach guard: the blitz cast completed, and its own discard cost
    // was paid, so the cost pipeline really ran.
    assert_eq!(runner.state().stack.len(), 1, "Sabin must be on the stack");
    assert_eq!(runner.state().objects[&filler].zone, Zone::Graveyard);

    assert_eq!(
        runner.state().objects[&land].zone,
        Zone::Battlefield,
        "Broodship is not the permission this cast uses, so its land-sacrifice \
         rider must not be charged"
    );
    assert!(
        !runner
            .state()
            .graveyard_cast_permissions_used
            .contains(&broodship),
        "Broodship's once-per-turn slot must stay unspent"
    );
}

/// Control for the test above: with Broodship as the ONLY permission, it is
/// the elected authority, so its land sacrifice IS charged and its slot IS
/// spent. Caldaia Guardian has no graveyard permission of its own.
#[test]
fn blitz_under_broodship_alone_charges_its_land_sacrifice() {
    let mut scenario = GameScenario::new();
    scenario.at_phase(Phase::PreCombatMain);
    let broodship = scenario
        .add_artifact_from_oracle(P0, "Exploration Broodship", EXPLORATION_BROODSHIP)
        .id();
    let land = scenario.add_basic_land(P0, ManaColor::Green);
    let guardian = scenario
        .add_creature_to_graveyard(P0, "Caldaia Guardian", 4, 3)
        .with_mana_cost(ManaCost::Cost {
            generic: 3,
            shards: vec![ManaCostShard::Green],
        })
        .with_keyword(caldaia_blitz())
        .id();
    let mut runner = scenario.build();
    fill_mana(&mut runner, ManaType::Green);

    cast_from_graveyard(&mut runner, guardian).expect("graveyard cast must be legal");
    runner
        .act(GameAction::ChooseAlternativeCast {
            choice: AlternativeCastDecision::Alternative,
        })
        .expect("choosing blitz must be legal");
    pay_offered_costs(&mut runner);

    assert_eq!(
        runner.state().stack.len(),
        1,
        "Caldaia must be on the stack"
    );
    assert_eq!(
        runner.state().objects[&land].zone,
        Zone::Graveyard,
        "Broodship admits this cast, so its land-sacrifice rider must be paid"
    );
    assert!(
        runner
            .state()
            .graveyard_cast_permissions_used
            .contains(&broodship),
        "Broodship's once-per-turn slot must be spent"
    );
}

/// Muldrotha, Encroaching Mycosynth and a graveyard Boon Satyr: a board where
/// a card cast for its own alternative cost would need a permanent-type slot
/// choice the engine can't prompt for yet.
fn mycosynth_muldrotha_board(scenario: &mut GameScenario) -> ObjectId {
    scenario.at_phase(Phase::PreCombatMain);
    add_permission_source(
        scenario,
        "Muldrotha, the Gravetide",
        MULDROTHA,
        &["Elemental", "Avatar"],
    );
    scenario.add_artifact_from_oracle(P0, "Encroaching Mycosynth", ENCROACHING_MYCOSYNTH);
    let mut builder = scenario.add_creature_to_graveyard(P0, "Boon Satyr", 4, 2);
    builder.with_mana_cost(ManaCost::Cost {
        generic: 1,
        shards: vec![ManaCostShard::Green, ManaCostShard::Green],
    });
    builder.with_subtypes(vec!["Satyr"]);
    builder.from_oracle_text_with_keywords(&["Flash", "Bestow"], BOON_SATYR);
    builder.id()
}

/// CR 110.4 + CR 702.103b: under Encroaching Mycosynth a bestowed Boon Satyr is
/// an artifact AND an enchantment spell, so Muldrotha offers two slots and the
/// player would have to choose one. That choice has no prompt on the
/// alternative-cost path yet, so Muldrotha can't be used for the bestow cast,
/// and the cast is refused before any cost is paid, rather than finalizing a
/// cast that spends no slot (or tripping the finalize-time slot assertion).
#[test]
fn bestow_needing_a_permanent_type_choice_is_refused_not_finalized() {
    let mut scenario = GameScenario::new();
    let satyr = mycosynth_muldrotha_board(&mut scenario);
    let mut runner = scenario.build();
    // Boon Satyr is an Enchantment Creature. The graveyard builder seeds only
    // Creature, so add Enchantment to both the current and base type lines.
    {
        let obj = runner.state_mut().objects.get_mut(&satyr).unwrap();
        for types in [
            &mut obj.card_types.core_types,
            &mut obj.base_card_types.core_types,
        ] {
            if !types.contains(&CoreType::Enchantment) {
                types.push(CoreType::Enchantment);
            }
        }
    }
    engine::game::layers::flush_layers(runner.state_mut());
    fill_mana(&mut runner, ManaType::Green);

    // Positive reach guard: Mycosynth really made the graveyard card an artifact,
    // which is what gives the bestowed spell its second slot.
    assert!(
        runner.state().objects[&satyr]
            .card_types
            .core_types
            .contains(&CoreType::Artifact),
        "Encroaching Mycosynth must make the graveyard Satyr an artifact, types: {:?}",
        runner.state().objects[&satyr].card_types.core_types
    );

    let result = cast_from_graveyard(&mut runner, satyr);
    assert!(
        result.is_err(),
        "the bestow cast must be refused while its slot choice has no prompt, got {result:?}"
    );
    assert!(
        runner.state().stack.is_empty(),
        "nothing may reach the stack"
    );
    let obj = &runner.state().objects[&satyr];
    assert_eq!(obj.zone, Zone::Graveyard);
    assert!(
        obj.card_types.core_types.contains(&CoreType::Creature),
        "the refused bestow must leave the card in its printed creature form, types: {:?}",
        obj.card_types.core_types
    );
    assert!(runner
        .state()
        .graveyard_cast_permissions_used_per_type
        .is_empty());
}

/// CR 110.4: the Blitz counterpart. Under Encroaching Mycosynth, Caldaia Guardian
/// is an artifact creature, so a blitz cast under Muldrotha would need the
/// slot choice too. Blitz is therefore not offered through Muldrotha, and the
/// printed-cost cast goes to the existing permanent-type slot prompt instead.
#[test]
fn blitz_needing_a_permanent_type_choice_is_not_offered() {
    let mut scenario = GameScenario::new();
    scenario.at_phase(Phase::PreCombatMain);
    add_permission_source(
        &mut scenario,
        "Muldrotha, the Gravetide",
        MULDROTHA,
        &["Elemental", "Avatar"],
    );
    scenario.add_artifact_from_oracle(P0, "Encroaching Mycosynth", ENCROACHING_MYCOSYNTH);
    let guardian = scenario
        .add_creature_to_graveyard(P0, "Caldaia Guardian", 4, 3)
        .with_mana_cost(ManaCost::Cost {
            generic: 3,
            shards: vec![ManaCostShard::Green],
        })
        .with_keyword(caldaia_blitz())
        .id();
    let mut runner = scenario.build();
    engine::game::layers::flush_layers(runner.state_mut());
    fill_mana(&mut runner, ManaType::Green);
    assert!(
        runner.state().objects[&guardian]
            .card_types
            .core_types
            .contains(&CoreType::Artifact),
        "Encroaching Mycosynth must make the graveyard Caldaia an artifact creature"
    );

    let waiting = cast_from_graveyard(&mut runner, guardian)
        .expect("the printed-cost graveyard cast must still be legal");
    assert!(
        matches!(waiting, WaitingFor::ChoosePermanentTypeSlot { .. }),
        "blitz must not be offered through Muldrotha here, so the printed cast \
         goes to the slot prompt, got {waiting:?}"
    );
}
