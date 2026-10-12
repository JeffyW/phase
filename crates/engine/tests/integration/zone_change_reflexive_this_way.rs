//! CR 603.12: the "When <zone change> this way, <body>" reflexive triggered
//! ability class (#9762). The head names a zone change an earlier instruction
//! of the same resolution performed; the reflexive is created after it, its
//! targets are chosen as it is put on the stack (CR 603.3d), and its body reads
//! the moved objects through referents the stack object carries.
//!
//! Shapes the engine cannot carry fail closed at parse time (the gate's event
//! must come from the instruction just before it; the body's object reference
//! must not be an unbound parent target).

use std::collections::VecDeque;

use engine::game::scenario::{GameRunner, GameScenario, P0, P1};
use engine::parser::oracle::parse_oracle_text;
use engine::types::ability::{AbilityDefinition, Effect, TargetRef};
use engine::types::actions::GameAction;
use engine::types::counter::CounterType;
use engine::types::game_state::{
    CastPaymentMode, PersistedGameState, PersistedRestoreFinalization, StackEntryKind, WaitingFor,
};
use engine::types::identifiers::ObjectId;
use engine::types::mana::ManaCost;
use engine::types::phase::Phase;
use engine::types::zones::Zone;

/// What happened while driving one resolution to an empty stack.
#[derive(Default)]
struct Drive {
    /// Zone of the watched object at each target prompt, in order.
    target_prompts: Vec<Zone>,
    /// Cards offered at each `EffectZoneChoice`, in order.
    zone_choices: Vec<Vec<ObjectId>>,
    /// Distinct triggered abilities seen on top of the stack at a priority
    /// window.
    triggers_on_stack: usize,
    seen_triggers: Vec<ObjectId>,
}

struct Plan {
    watched: ObjectId,
    targets: VecDeque<Vec<TargetRef>>,
    picks: Vec<ObjectId>,
    accept_optional: bool,
}

impl Plan {
    fn new(watched: ObjectId) -> Self {
        Self {
            watched,
            targets: VecDeque::new(),
            picks: Vec::new(),
            accept_optional: true,
        }
    }
    fn target(mut self, targets: Vec<TargetRef>) -> Self {
        self.targets.push_back(targets);
        self
    }
    fn picks(mut self, picks: Vec<ObjectId>) -> Self {
        self.picks = picks;
        self
    }
    fn decline_optional(mut self) -> Self {
        self.accept_optional = false;
        self
    }
}

fn pick(plan: &Plan, offered: &[ObjectId], count: usize) -> Vec<ObjectId> {
    plan.picks
        .iter()
        .copied()
        .filter(|card| offered.contains(card))
        .take(count)
        .collect()
}

fn drive(r: &mut GameRunner, mut plan: Plan) -> Drive {
    let mut out = Drive::default();
    for _ in 0..80 {
        if std::env::var_os("ZCR_TRACE").is_some() {
            let text = format!("{:?}", r.state().waiting_for);
            eprintln!("TRACE {}", &text[..text.len().min(400)]);
        }
        let result = match r.state().waiting_for.clone() {
            WaitingFor::TargetSelection { .. } | WaitingFor::TriggerTargetSelection { .. } => {
                out.target_prompts
                    .push(r.state().objects[&plan.watched].zone);
                let targets = plan.targets.pop_front().unwrap_or_default();
                r.act(GameAction::SelectTargets { targets })
            }
            WaitingFor::DiscardChoice { cards, count, .. } => {
                let chosen = pick(&plan, &cards, count);
                r.act(GameAction::SelectCards { cards: chosen })
            }
            WaitingFor::EffectZoneChoice { cards, count, .. } => {
                out.zone_choices.push(cards.clone());
                let chosen = pick(&plan, &cards, count);
                r.act(GameAction::SelectCards { cards: chosen })
            }
            WaitingFor::DigChoice {
                cards, keep_count, ..
            } => {
                let chosen = pick(&plan, &cards, keep_count);
                r.act(GameAction::SelectCards { cards: chosen })
            }
            WaitingFor::ScryChoice { .. } => r.act(GameAction::SelectCards { cards: vec![] }),
            WaitingFor::OptionalEffectChoice { .. } => r.act(GameAction::DecideOptionalEffect {
                accept: plan.accept_optional,
            }),
            WaitingFor::Priority { .. } if r.state().stack.is_empty() => break,
            WaitingFor::Priority { .. } => {
                if let Some(entry) =
                    r.state().stack.back().filter(|entry| {
                        matches!(entry.kind, StackEntryKind::TriggeredAbility { .. })
                    })
                {
                    if !out.seen_triggers.contains(&entry.id) {
                        out.seen_triggers.push(entry.id);
                        out.triggers_on_stack += 1;
                    }
                }
                r.act(GameAction::PassPriority)
            }
            other => panic!("unexpected prompt {other:?}"),
        };
        result.expect("action accepted");
    }
    out
}

fn cast(r: &mut GameRunner, spell: ObjectId) {
    let card_id = r.state().objects[&spell].card_id;
    r.act(GameAction::CastSpell {
        object_id: spell,
        card_id,
        targets: vec![],
        payment_mode: CastPaymentMode::Auto,
    })
    .expect("cast");
}

fn counters(r: &GameRunner, id: ObjectId) -> u32 {
    r.state().objects[&id]
        .counters
        .get(&CounterType::Plus1Plus1)
        .copied()
        .unwrap_or(0)
}

fn spell_ability(text: &str, name: &str) -> AbilityDefinition {
    let parsed = parse_oracle_text(text, name, &[], &["Sorcery".to_string()], &[]);
    parsed
        .abilities
        .into_iter()
        .next()
        .expect("one spell ability")
}

/// The first `Unimplemented` name along the ability's sub chain.
fn unimplemented_name(def: &AbilityDefinition) -> Option<String> {
    let mut node = Some(def);
    while let Some(current) = node {
        if let Effect::Unimplemented { name, .. } = current.effect.as_ref() {
            return Some(name.clone());
        }
        node = current.sub_ability.as_deref();
    }
    None
}

// ---------------------------------------------------------------------------
// Discard heads
// ---------------------------------------------------------------------------

const GLACIAL_DRAGONHUNT: &str = "Draw a card, then you may discard a card. When you discard a nonland card this way, Glacial Dragonhunt deals 3 damage to target creature.";

fn glacial_fixture() -> (GameRunner, ObjectId, ObjectId, ObjectId) {
    let mut s = GameScenario::new();
    s.at_phase(Phase::PreCombatMain);
    let spell = s
        .add_spell_to_hand_from_oracle(P0, "Glacial Dragonhunt", false, GLACIAL_DRAGONHUNT)
        .id();
    let three = s
        .add_spell_to_hand(P0, "Three Drop", false)
        .with_mana_cost(ManaCost::generic(3))
        .id();
    s.add_card_to_library_top(P0, "Library Card");
    let opp = s.add_creature(P1, "Opp Giant", 4, 4).id();
    s.add_creature(P1, "Opp Cub", 1, 1);
    (s.build(), spell, three, opp)
}

/// CR 603.12 + CR 603.3d: Glacial Dragonhunt announces no target as it is cast;
/// the reflexive asks for "target creature" after the nonland discard.
#[test]
fn glacial_dragonhunt_targets_after_the_discard() {
    let (mut r, spell, three, opp) = glacial_fixture();
    cast(&mut r, spell);
    let out = drive(
        &mut r,
        Plan::new(three)
            .target(vec![TargetRef::Object(opp)])
            .picks(vec![three]),
    );
    assert_eq!(
        out.target_prompts,
        vec![Zone::Graveyard],
        "one target prompt, after the card was discarded"
    );
    assert_eq!(r.state().objects[&opp].damage_marked, 3);
}

/// CR 603.12: declining the optional discard means the event never happened.
#[test]
fn glacial_dragonhunt_decline_creates_no_reflexive() {
    let (mut r, spell, three, opp) = glacial_fixture();
    cast(&mut r, spell);
    let out = drive(&mut r, Plan::new(three).decline_optional());
    assert!(out.target_prompts.is_empty(), "no target prompt");
    assert_eq!(
        r.state().objects[&three].zone,
        Zone::Hand,
        "reach: nothing discarded"
    );
    assert_eq!(r.state().objects[&opp].damage_marked, 0);
}

/// Borborygmos and Fblthp's body: "twice that much" reads the number discarded
/// this way after the reflexive leaves the creating resolution.
#[test]
fn borborygmos_body_twice_that_much_survives_the_stack() {
    let mut s = GameScenario::new();
    s.at_phase(Phase::PreCombatMain);
    let spell = s
        .add_spell_to_hand_from_oracle(
            P0,
            "Probe",
            false,
            "Draw a card, then you may discard any number of land cards. When you discard one or more cards this way, Probe deals twice that much damage to target creature.",
        )
        .id();
    let l1 = s.add_land_to_hand(P0, "Land One").id();
    let l2 = s.add_land_to_hand(P0, "Land Two").id();
    s.add_card_to_library_top(P0, "Library Card");
    let opp = s.add_creature(P1, "Opp Giant", 9, 9).id();
    s.add_creature(P1, "Opp Cub", 1, 1);
    let mut r = s.build();
    cast(&mut r, spell);
    let out = drive(
        &mut r,
        Plan::new(l1)
            .target(vec![TargetRef::Object(opp)])
            .picks(vec![l1, l2]),
    );
    assert_eq!(out.target_prompts, vec![Zone::Graveyard]);
    assert_eq!(
        r.state().objects[&opp].damage_marked,
        4,
        "twice the two discarded"
    );
}

// ---------------------------------------------------------------------------
// Exile heads
// ---------------------------------------------------------------------------

const HIDETSUGU_BODY: &str = "Exile the top card of your library. You may play that card this turn. When you exile a nonland card this way, Probe deals damage equal to the exiled card's mana value to any target.";

/// Hidetsugu, Devouring Chaos's ability body: the play permission between the
/// exile and the head is not the exiling instruction (CR 608.2c), so the
/// reflexive still sees the exiled card, targets after the exile, and deals
/// that card's mana value.
#[test]
fn hidetsugu_body_reflexive_reads_the_exile_across_the_play_permission() {
    let mut s = GameScenario::new();
    s.at_phase(Phase::PreCombatMain);
    let spell = s
        .add_spell_to_hand_from_oracle(P0, "Probe", false, HIDETSUGU_BODY)
        .id();
    let top = s
        .add_spell_to_library_top(P0, "Three Drop", false)
        .with_mana_cost(ManaCost::generic(3))
        .id();
    let mut r = s.build();
    cast(&mut r, spell);
    let out = drive(&mut r, Plan::new(top).target(vec![TargetRef::Player(P1)]));
    assert_eq!(out.target_prompts, vec![Zone::Exile]);
    assert_eq!(r.life(P1), 17);
    assert!(
        !r.state().objects[&top].casting_permissions.is_empty(),
        "the play permission still stands"
    );
}

/// The "If" connector over the same ledger: a resolution-time condition after
/// the play permission (announced with the spell, CR 601.2c) now resolves.
#[test]
fn if_exiled_this_way_after_a_play_permission_resolves() {
    let mut s = GameScenario::new();
    s.at_phase(Phase::PreCombatMain);
    let spell = s
        .add_spell_to_hand_from_oracle(
            P0,
            "Probe",
            false,
            "Exile the top card of your library. You may play that card this turn. If you exiled a nonland card this way, Probe deals 2 damage to target player.",
        )
        .id();
    let top = s
        .add_spell_to_library_top(P0, "Three Drop", false)
        .with_mana_cost(ManaCost::generic(3))
        .id();
    let mut r = s.build();
    cast(&mut r, spell);
    let out = drive(&mut r, Plan::new(top).target(vec![TargetRef::Player(P1)]));
    assert_eq!(
        out.target_prompts,
        vec![Zone::Library],
        "announced with the spell"
    );
    assert_eq!(r.state().objects[&top].zone, Zone::Exile);
    assert_eq!(r.life(P1), 18);
}

const AGATHA_BODY: &str = "Exile target card from a graveyard. When a creature card is exiled this way, put a +1/+1 counter on target creature you control.";

/// Agatha's Soul Cauldron's ability body: the parent announces its own target;
/// the reflexive announces a second one after the exile.
#[test]
fn agatha_body_reflexive_targets_after_a_creature_card_is_exiled() {
    for exile_a_creature in [true, false] {
        let mut s = GameScenario::new();
        s.at_phase(Phase::PreCombatMain);
        let spell = s
            .add_spell_to_hand_from_oracle(P0, "Probe", false, AGATHA_BODY)
            .id();
        let dead = if exile_a_creature {
            s.add_creature_to_graveyard(P1, "Dead Bear", 2, 2).id()
        } else {
            s.add_spell_to_graveyard(P1, "Old Sorcery", false).id()
        };
        // A second graveyard card, so the spell's own target is a real choice.
        s.add_land_to_graveyard(P1, "Old Land");
        let bear = s.add_creature(P0, "Bear", 2, 2).id();
        s.add_creature(P0, "Cub", 1, 1);
        let mut r = s.build();
        cast(&mut r, spell);
        let out = drive(
            &mut r,
            Plan::new(dead)
                .target(vec![TargetRef::Object(dead)])
                .target(vec![TargetRef::Object(bear)]),
        );
        assert_eq!(r.state().objects[&dead].zone, Zone::Exile, "reach: exiled");
        if exile_a_creature {
            assert_eq!(out.target_prompts, vec![Zone::Graveyard, Zone::Exile]);
            assert_eq!(counters(&r, bear), 1);
        } else {
            // CR 603.12: a noncreature card exiled is not the trigger event.
            assert_eq!(out.target_prompts, vec![Zone::Graveyard]);
            assert_eq!(counters(&r, bear), 0);
        }
    }
}

/// Augusta, Order Returned (fail closed): "each player exiles a card from their
/// graveyard" resolves its gated clause once per player, so the single CR
/// 603.12 event "one or more nonland cards are exiled this way" would create
/// one reflexive per player. Measured on the spell form: the first reflexive's
/// target prompt opened while the opponent's card was still in the graveyard.
#[test]
fn augusta_reflexive_over_a_player_iteration_fails_closed() {
    let parsed = parse_oracle_text(
        "Whenever Augusta attacks, each player exiles a card from their graveyard. When one or more nonland cards are exiled this way, put that many +1/+1 counters on target attacking creature.",
        "Augusta, Order Returned",
        &[],
        &["Creature".to_string()],
        &[],
    );
    let execute = parsed.triggers[0].execute.as_deref().expect("trigger body");
    assert!(
        execute.player_scope.is_some(),
        "reach guard: the producer iterates players"
    );
    assert_eq!(
        unimplemented_name(execute).as_deref(),
        Some("zone_change_reflexive_event_source"),
        "{execute:?}"
    );
}

// ---------------------------------------------------------------------------
// Sacrifice heads ("that many")
// ---------------------------------------------------------------------------

/// Probe board 3, Ravenous Rotbelly (fail closed): its reflexive body iterates
/// over players ("each opponent sacrifices that many"), and a player-scoped
/// reflexive body resolves inside the creating resolution instead of waiting
/// on the stack as its own object, so the printed card is not claimed. (Its
/// "up to three Zombies" head also offers only one; listed separately.)
#[test]
fn rotbelly_player_scoped_reflexive_body_fails_closed() {
    let parsed = parse_oracle_text(
        "When this creature enters, you may sacrifice up to three Zombies. When you sacrifice one or more Zombies this way, each opponent sacrifices that many creatures of their choice.",
        "Ravenous Rotbelly",
        &[],
        &["Creature".to_string()],
        &[],
    );
    let execute = parsed.triggers[0].execute.as_deref().expect("ETB body");
    assert!(
        matches!(execute.effect.as_ref(), Effect::Sacrifice { .. }),
        "reach guard: the producer is the printed sacrifice"
    );
    assert_eq!(
        unimplemented_name(execute).as_deref(),
        Some("zone_change_reflexive_scoped_body"),
        "{execute:?}"
    );
}

/// Nyssa of Traken's attack-trigger body: "that many" bounds both the
/// reflexive's target count (chosen as it goes on the stack) and its draw.
#[test]
fn nyssa_body_that_many_bounds_targets_and_draw() {
    let mut s = GameScenario::new();
    s.at_phase(Phase::PreCombatMain);
    let spell = s
        .add_spell_to_hand_from_oracle(
            P0,
            "Probe",
            false,
            "Sacrifice any number of artifacts. When you sacrifice one or more artifacts this way, tap up to that many target creatures and draw that many cards.",
        )
        .id();
    let a1 = s.add_creature(P0, "Relic One", 0, 1).as_artifact().id();
    let a2 = s.add_creature(P0, "Relic Two", 0, 1).as_artifact().id();
    let o1 = s.add_creature(P1, "Opp One", 1, 1).id();
    let o2 = s.add_creature(P1, "Opp Two", 1, 1).id();
    s.add_creature(P1, "Opp Three", 1, 1);
    s.with_library_top(P0, &["Draw One", "Draw Two", "Draw Three"]);
    let mut r = s.build();
    let hand_before = r.state().players[0].hand.len();
    cast(&mut r, spell);
    let out = drive(
        &mut r,
        Plan::new(a1)
            .picks(vec![a1, a2])
            .target(vec![TargetRef::Object(o1), TargetRef::Object(o2)]),
    );
    assert_eq!(
        out.target_prompts,
        vec![Zone::Graveyard],
        "targets after the sacrifice"
    );
    assert!(
        out.triggers_on_stack >= 1,
        "the reflexive went on the stack"
    );
    assert!(r.state().objects[&o1].tapped && r.state().objects[&o2].tapped);
    assert_eq!(
        r.state().players[0].hand.len(),
        hand_before - 1 + 2,
        "drew that many (two); the cast spell left the hand"
    );
}

// ---------------------------------------------------------------------------
// Battlefield-entry heads
// ---------------------------------------------------------------------------

/// Adaptive Armorer / Masterpiece Vault shape ("When an Equipment enters this
/// way, attach it to target creature you control"), from a production
/// put-onto-battlefield producer: the moved Equipment is the attachment, the
/// host is the reflexive's own target chosen after the entry.
#[test]
fn entered_equipment_attaches_to_the_reflexive_target() {
    let mut s = GameScenario::new();
    s.at_phase(Phase::PreCombatMain);
    let spell = s
        .add_spell_to_hand_from_oracle(
            P0,
            "Probe",
            false,
            "Put an Equipment card from your hand onto the battlefield. When an Equipment enters this way, attach it to target creature you control.",
        )
        .id();
    let equipment = s
        .add_artifact_to_hand_from_oracle(P0, "Hand Equipment", "Equip {2}")
        .with_subtypes(vec!["Equipment"])
        .id();
    // A second Equipment in hand, so the put-onto-the-battlefield instruction
    // pauses for its choice and the reflexive is created on the resumed path.
    let other_equipment = s
        .add_artifact_to_hand_from_oracle(P0, "Other Hand Equipment", "Equip {2}")
        .with_subtypes(vec!["Equipment"])
        .id();
    let bear = s.add_creature(P0, "Bear", 2, 2).id();
    s.add_creature(P0, "Cub", 1, 1);
    let mut r = s.build();
    cast(&mut r, spell);
    let out = drive(
        &mut r,
        Plan::new(equipment)
            .picks(vec![equipment])
            .target(vec![TargetRef::Object(bear)]),
    );
    assert_eq!(
        out.zone_choices.len(),
        1,
        "reach: the producer paused for its choice"
    );
    assert_eq!(
        r.state().objects[&other_equipment].zone,
        Zone::Hand,
        "only the chosen Equipment moved"
    );
    assert_eq!(
        out.target_prompts,
        vec![Zone::Battlefield],
        "host chosen after entry"
    );
    assert_eq!(
        r.state().objects[&equipment].attached_to,
        Some(engine::game::game_object::AttachTarget::Object(bear))
    );
}

/// Probe boards 1 and 2 (fail closed): Adaptive Armorer and Masterpiece Vault
/// put the drafted card onto the battlefield through the spellbook-draft
/// resume, which does not publish the zone-change ledger the gate reads, so
/// the reflexive could never be created; both printed cards are not claimed.
/// (Masterpiece Vault's "up to one target" host is moot behind that.)
#[test]
fn spellbook_draft_entry_reflexives_fail_closed() {
    for (name, text, types) in [
        (
            "Adaptive Armorer",
            "Vigilance\nWhen this creature enters, draft a card from Adaptive Armorer's spellbook and put it onto the battlefield. When an Equipment enters this way, attach it to target creature you control.",
            "Creature",
        ),
        (
            "Masterpiece Vault",
            "When this artifact is put into a graveyard from the battlefield, draft a card from Masterpiece Vault's spellbook and put it onto the battlefield. When an Equipment enters this way, attach it to up to one target creature you control.\n{5}: Sacrifice this artifact.",
            "Artifact",
        ),
    ] {
        let parsed = parse_oracle_text(text, name, &[], &[types.to_string()], &[]);
        let execute = parsed
            .triggers
            .iter()
            .find_map(|trigger| trigger.execute.as_deref())
            .expect("trigger body");
        assert!(
            matches!(execute.effect.as_ref(), Effect::DraftFromSpellbook { .. }),
            "reach guard: {name}'s producer is the spellbook draft"
        );
        assert_eq!(
            unimplemented_name(execute).as_deref(),
            Some("zone_change_reflexive_event_source"),
            "{name}: {execute:?}"
        );
    }
}

/// Probe board 4 (CR 603.12a): "When you exile a card this way" counts one
/// occurrence, so after an instruction that exiles two cards it would trigger
/// twice, while the creation gate makes one trigger: fails closed. The
/// aggregate head ("one or more cards") is one event and triggers once.
#[test]
fn single_occurrence_head_after_a_multi_card_producer_fails_closed() {
    let single = spell_ability(
        "Exile the top two cards of your library. When you exile a card this way, you gain 1 life.",
        "Probe",
    );
    assert!(
        matches!(single.effect.as_ref(), Effect::ExileTop { .. }),
        "reach guard: the two-card exile"
    );
    assert_eq!(
        unimplemented_name(&single).as_deref(),
        Some("zone_change_reflexive_occurrence"),
        "{single:?}"
    );

    let mut s = GameScenario::new();
    s.at_phase(Phase::PreCombatMain);
    let spell = s
        .add_spell_to_hand_from_oracle(
            P0,
            "Probe",
            false,
            "Exile the top two cards of your library. When you exile one or more cards this way, you gain 1 life.",
        )
        .id();
    let first = s.add_card_to_library_top(P0, "First");
    s.add_card_to_library_top(P0, "Second");
    let mut r = s.build();
    let life = r.life(P0);
    cast(&mut r, spell);
    let out = drive(&mut r, Plan::new(first));
    assert_eq!(r.state().objects[&first].zone, Zone::Exile, "reach: exiled");
    assert_eq!(out.triggers_on_stack, 1, "one reflexive on the stack");
    assert_eq!(r.life(P0), life + 1, "the aggregate head triggers once");
}

/// CR 603.12: the reflexive survives a save/restore taken while its PRODUCER
/// is paused on the discard choice; after the restore the choice is answered
/// and the reflexive is created and targets as usual.
#[test]
fn reflexive_survives_save_and_restore_at_the_producers_discard_choice() {
    let mut s = GameScenario::new();
    s.at_phase(Phase::PreCombatMain);
    let spell = s
        .add_spell_to_hand_from_oracle(P0, "Glacial Dragonhunt", false, GLACIAL_DRAGONHUNT)
        .id();
    let three = s
        .add_spell_to_hand(P0, "Three Drop", false)
        .with_mana_cost(ManaCost::generic(3))
        .id();
    s.add_spell_to_hand(P0, "Two Drop", false)
        .with_mana_cost(ManaCost::generic(2));
    s.add_card_to_library_top(P0, "Library Card");
    let opp = s.add_creature(P1, "Opp Giant", 4, 4).id();
    s.add_creature(P1, "Opp Cub", 1, 1);
    let mut r = s.build();
    cast(&mut r, spell);
    for _ in 0..8 {
        match r.state().waiting_for.clone() {
            WaitingFor::DiscardChoice { .. } => break,
            WaitingFor::OptionalEffectChoice { .. } => {
                r.act(GameAction::DecideOptionalEffect { accept: true })
                    .expect("accept the optional discard");
            }
            WaitingFor::Priority { .. } => {
                r.act(GameAction::PassPriority).expect("pass");
            }
            other => panic!("unexpected prompt {other:?}"),
        }
    }
    assert!(
        matches!(r.state().waiting_for, WaitingFor::DiscardChoice { .. }),
        "reach: the producer is paused on its choice"
    );
    let json = serde_json::to_string(&PersistedGameState::capture(r.state().clone()))
        .expect("state serializes");
    let state = serde_json::from_str::<PersistedGameState>(&json)
        .expect("persisted state decodes")
        .prepare_for_restore(PersistedRestoreFinalization::DeferUntilRehydrated)
        .expect("persisted state is admissible")
        .finalize_after_rehydration(|_| Ok(()))
        .expect("restored state is publishable");
    let mut restored = GameRunner::from_state(state);
    let out = drive(
        &mut restored,
        Plan::new(three)
            .picks(vec![three])
            .target(vec![TargetRef::Object(opp)]),
    );
    assert_eq!(out.target_prompts, vec![Zone::Graveyard]);
    assert_eq!(restored.state().objects[&opp].damage_marked, 3);
}

/// CR 603.12 + CR 603.3d (fail closed): printed class members the reflexive
/// stack object cannot carry — Hans Eriksson (its event is put onto the
/// battlefield by a branch of the reveal, and "it fights" was left as the
/// parent's target), Three Dog ("a copy of that Aura" has no copy source), and
/// Saruman of Many Colors ("each opponent mills" resolves its gate once per
/// opponent).
#[test]
fn reflexive_with_an_object_the_stack_cannot_carry_fails_closed() {
    for (name, expected_gap, text) in [
        (
            "Hans Eriksson",
            "zone_change_reflexive_event_source",
            "Whenever Hans Eriksson attacks, reveal the top card of your library. If it's a creature card, put it onto the battlefield tapped and attacking defending player or a planeswalker they control. Otherwise, put that card into your hand. When you put a creature card onto the battlefield this way, it fights Hans Eriksson.",
        ),
        (
            "Three Dog, Galaxy News DJ",
            "zone_change_reflexive_unbound_referent",
            "Whenever you attack, you may pay {2} and sacrifice an Aura attached to Three Dog. When you sacrifice an Aura this way, for each other attacking creature you control, create a token that's a copy of that Aura attached to that creature.",
        ),
        (
            "Saruman of Many Colors",
            "zone_change_reflexive_event_source",
            "Whenever you cast your second spell each turn, each opponent mills two cards. When one or more cards are milled this way, exile target enchantment, instant, or sorcery card with equal or lesser mana value than that spell from an opponent's graveyard. Copy the exiled card. You may cast the copy without paying its mana cost.",
        ),
    ] {
        let parsed = parse_oracle_text(text, name, &[], &["Creature".to_string()], &[]);
        let json = serde_json::to_string(&parsed.triggers).unwrap();
        assert!(json.contains(expected_gap), "{name}: {json}");
        assert!(!json.contains("WhenYouDo"), "{name}: no reflexive gate is claimed: {json}");
    }
}

// ---------------------------------------------------------------------------
// Fail-closed shapes
// ---------------------------------------------------------------------------

/// CR 608.2c: the gate reads the zone change the instruction just before it
/// published. A different instruction in between ("then draw", "then seek"),
/// a cost-shaped discard under a sacrifice, or a delayed sacrifice leaves the
/// gate reading the wrong cards, so the reflexive is not claimed.
#[test]
fn reflexive_whose_event_is_not_the_preceding_instruction_fails_closed() {
    for (name, text) in [
        (
            "Fiery Encore",
            "Discard a card, then draw a card. When you discard a nonland card this way, Fiery Encore deals damage equal to that card's mana value to target creature or planeswalker.",
        ),
        (
            "Flamethrower Sonata",
            "Discard a card, then draw a card. When you discard an instant or sorcery card this way, Flamethrower Sonata deals damage equal to that card's mana value to target creature or planeswalker you don't control.",
        ),
        (
            "Summon: Kujata",
            "Discard a card, then draw two cards. When you discard a card this way, this creature deals damage equal to that card's mana value to each opponent.",
        ),
    ] {
        let def = spell_ability(text, name);
        assert_eq!(
            unimplemented_name(&def).as_deref(),
            Some("zone_change_reflexive_event_source"),
            "{name}: {def:?}"
        );
    }
    for (name, text, types) in [
        (
            "Chomping Mastasaur",
            "Whenever this creature enters or attacks, discard a card, then seek a nonland card. When you discard a card this way, this creature deals damage equal to the discarded card's mana value to any target.",
            "Creature",
        ),
        (
            "Argentum Masticore",
            "At the beginning of your upkeep, sacrifice this creature unless you discard a card. When you discard a card this way, destroy target nonland permanent an opponent controls with mana value less than or equal to the mana value of the discarded card.",
            "Creature",
        ),
        (
            "Goblin Crash Pilot",
            "Whenever this creature crews a Vehicle, that Vehicle gains haste until end of turn. At the beginning of the next end step, sacrifice that Vehicle. When you sacrifice a creature this way, it deals damage equal to its power to any target.",
            "Creature",
        ),
    ] {
        let parsed = parse_oracle_text(text, name, &[], &[types.to_string()], &[]);
        let execute = parsed.triggers[0].execute.as_deref().expect("trigger body");
        assert_eq!(
            unimplemented_name(execute).as_deref(),
            Some("zone_change_reflexive_event_source"),
            "{name}: {execute:?}"
        );
    }
}

/// CR 603.3d: a reflexive created by a zone change has no announced parent
/// target, so a body whose recipient was rebound to the parent's target can
/// never choose one. Not claimed.
#[test]
fn reflexive_with_an_unbound_recipient_fails_closed() {
    let parsed = parse_oracle_text(
        "When this creature enters, draw two cards, then discard two cards. When you discard one or more nonland cards this way, tap up to that many target creatures and put a stun counter on each of them.",
        "Seasoned Cryomancer",
        &[],
        &["Creature".to_string()],
        &[],
    );
    let execute = parsed.triggers[0].execute.as_deref().expect("ETB body");
    assert!(
        matches!(execute.effect.as_ref(), Effect::Draw { .. }),
        "reach guard: the printed ETB body"
    );
    assert_eq!(
        unimplemented_name(execute).as_deref(),
        Some("zone_change_reflexive_unbound_referent")
    );
}

/// CR 603.12 + CR 603.4: a zone-change intervening "if" on a reflexive would
/// share the root with a zone-change trigger event; the role is not
/// representable, so it fails closed instead of being settled as the event.
#[test]
fn zone_change_intervening_if_on_a_reflexive_fails_closed() {
    let def = spell_ability(
        "Exile target card from a graveyard. When you do, if a creature card was exiled this way, you gain 3 life.",
        "Probe",
    );
    assert_eq!(
        unimplemented_name(&def).as_deref(),
        Some("when_you_do_guard"),
        "{def:?}"
    );
}
