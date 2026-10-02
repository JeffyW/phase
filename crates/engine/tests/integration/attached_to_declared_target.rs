//! "<type> attached to that creature" — the declared-slot attachment referent.
//!
//! Light of Judgment (FIN) — "Light of Judgment deals 6 damage to target
//! creature. Destroy up to one Equipment attached to that creature." — and its
//! class: Turn to Slag, Blastfire Bolt, Fires of Mount Doom ("Destroy all
//! Equipment attached to that creature") and Fiery Annihilation ("Exile up to
//! one target Equipment attached to that creature. If that creature would die
//! this turn, exile it instead.").
//!
//! "that creature" names the object announced for an earlier declared target
//! slot, so the relation lowers to
//! `FilterProp::AttachedTo { to: AttachmentReferent::DeclaredTarget { slot } }`
//! and the referent is read by slot, never as "the first object target".
//!
//! CR set (each verified against `docs/MagicCompRules.txt`):
//! CR 115.3 (one object may fill several "target" instances), CR 115.10a
//! (only the word "target" makes a target), CR 601.2c (announcing targets),
//! CR 608.2b (illegal targets; information about an illegal target is not
//! determined), CR 608.2c (instructions in order), CR 608.2d (choices made
//! while applying the effect), CR 608.2h (last known information), CR 614.1a
//! (replacement effects), CR 701.3a (attach), CR 701.8a/b (destroy;
//! indestructible), CR 704.3 (no state-based actions during resolution),
//! CR 400.7 (a moved object is a new object).
//!
//! Oracle text is verbatim from Scryfall.

use engine::game::combat::AttackTarget;
use engine::game::effects::attach;
use engine::game::game_object::AttachTarget;
use engine::game::scenario::{GameRunner, GameScenario, P0, P1};
use engine::game::zone_pipeline::{move_object_for_test, ZoneMoveRequest};
use engine::parser::oracle::parse_oracle_text;
use engine::types::ability::{
    AbilityDefinition, AttachmentReferent, Effect, FilterProp, TargetFilter, TargetRef,
};
use engine::types::actions::GameAction;
use engine::types::events::GameEvent;
use engine::types::game_state::WaitingFor;
use engine::types::identifiers::ObjectId;
use engine::types::keywords::Keyword;
use engine::types::mana::ManaCost;
use engine::types::phase::Phase;
use engine::types::zones::Zone;

pub(crate) const LIGHT_OF_JUDGMENT: &str =
    "Light of Judgment deals 6 damage to target creature. Destroy up to one Equipment attached to that creature.";
pub(crate) const TURN_TO_SLAG: &str =
    "Turn to Slag deals 5 damage to target creature. Destroy all Equipment attached to that creature.";
pub(crate) const BLASTFIRE_BOLT: &str =
    "Blastfire Bolt deals 5 damage to target creature. Destroy all Equipment attached to that creature.";
pub(crate) const FIRES_OF_MOUNT_DOOM: &str = "When Fires of Mount Doom enters, it deals 2 damage to target creature an opponent controls. Destroy all Equipment attached to that creature.\n{2}{R}: Exile the top card of your library. You may play that card this turn. When you play a card this way, Fires of Mount Doom deals 2 damage to each player.";
pub(crate) const FIERY_ANNIHILATION: &str = "Fiery Annihilation deals 5 damage to target creature. Exile up to one target Equipment attached to that creature. If that creature would die this turn, exile it instead.";
const TREEFOLK_MYSTIC: &str = "Whenever this creature blocks or becomes blocked by a creature, destroy all Auras attached to that creature.";
const CORROSIVE_OOZE: &str = "Whenever this creature blocks or becomes blocked by an equipped creature, destroy all Equipment attached to that creature at end of combat.";
const SHACKLES_OF_TREACHERY: &str = "Gain control of target creature until end of turn. Untap that creature. Until end of turn, it gains haste and \"Whenever this creature deals damage, destroy target Equipment attached to it.\"";

fn types(core: &str) -> Vec<String> {
    vec![core.to_string()]
}

/// Every effect of `def`'s `sub_ability` line, head first.
pub(crate) fn chain(def: &AbilityDefinition) -> Vec<&Effect> {
    std::iter::successors(Some(def), |d| d.sub_ability.as_deref())
        .map(|d| &*d.effect)
        .collect()
}

fn unimplemented_names(defs: &[&AbilityDefinition]) -> Vec<String> {
    defs.iter()
        .flat_map(|d| chain(d))
        .filter_map(|e| match e {
            Effect::Unimplemented { name, .. } => Some(name.clone()),
            _ => None,
        })
        .collect()
}

/// The declared-slot referents carried anywhere on `filter`'s typed legs.
pub(crate) fn declared_slots(filter: &TargetFilter) -> Vec<usize> {
    let mut slots = Vec::new();
    fn walk(f: &TargetFilter, out: &mut Vec<usize>) {
        match f {
            TargetFilter::Typed(tf) => {
                for p in &tf.properties {
                    if let FilterProp::AttachedTo {
                        to: AttachmentReferent::DeclaredTarget { slot },
                    } = p
                    {
                        out.push(*slot);
                    }
                }
            }
            TargetFilter::Or { filters } | TargetFilter::And { filters } => {
                filters.iter().for_each(|x| walk(x, out))
            }
            _ => {}
        }
    }
    walk(filter, &mut slots);
    slots
}

/// SHAPE: Light of Judgment's second sentence is a resolution-time selection of
/// up to one Equipment attached to declared slot 0, then destruction of exactly
/// the chosen set (CR 115.10a + CR 608.2d).
#[test]
fn light_of_judgment_lowers_to_resolution_choice_of_slot_zero_attachments() {
    let parsed = parse_oracle_text(
        LIGHT_OF_JUDGMENT,
        "Light of Judgment",
        &[],
        &types("Instant"),
        &[],
    );
    assert_eq!(parsed.abilities.len(), 1);
    let effects = chain(&parsed.abilities[0]);
    assert!(
        unimplemented_names(&[&parsed.abilities[0]]).is_empty(),
        "reach guard: no gap may remain, got {effects:?}"
    );
    assert!(matches!(effects[0], Effect::DealDamage { .. }));
    let Effect::ChooseObjectsIntoTrackedSet {
        filter, min, max, ..
    } = effects[1]
    else {
        panic!("second node must be the resolution-time choice, got {effects:?}");
    };
    assert_eq!((*min, *max), (0, Some(1)), "up to one");
    assert_eq!(declared_slots(filter), vec![0]);
    assert!(
        matches!(
            effects[2],
            Effect::DestroyAll {
                target: TargetFilter::TrackedSet { .. },
                ..
            }
        ),
        "third node destroys exactly the chosen set, got {effects:?}"
    );
    assert!(
        parsed.abilities[0]
            .sub_ability
            .as_ref()
            .is_some_and(|s| s.multi_target.is_none()),
        "the untargeted choice must not leak a multi-target spec"
    );
}

/// SHAPE: the "all" siblings lower to a `DestroyAll` over slot 0's attachments.
#[test]
fn destroy_all_equipment_attached_to_that_creature_siblings() {
    for (oracle, name) in [
        (TURN_TO_SLAG, "Turn to Slag"),
        (BLASTFIRE_BOLT, "Blastfire Bolt"),
    ] {
        let parsed = parse_oracle_text(oracle, name, &[], &types("Sorcery"), &[]);
        let effects = chain(&parsed.abilities[0]);
        assert!(
            unimplemented_names(&[&parsed.abilities[0]]).is_empty(),
            "{name}: reach guard, got {effects:?}"
        );
        let Effect::DestroyAll { target, .. } = effects[1] else {
            panic!("{name}: expected DestroyAll, got {effects:?}");
        };
        assert_eq!(declared_slots(target), vec![0], "{name}");
    }
}

/// SHAPE: the ETB trigger body binds "that creature" to the trigger's own
/// declared slot 0.
#[test]
fn fires_of_mount_doom_trigger_body_binds_slot_zero() {
    let parsed = parse_oracle_text(
        FIRES_OF_MOUNT_DOOM,
        "Fires of Mount Doom",
        &[],
        &types("Enchantment"),
        &[],
    );
    let execute = parsed.triggers[0]
        .execute
        .as_deref()
        .expect("ETB trigger body");
    let effects = chain(execute);
    assert!(
        unimplemented_names(&[execute]).is_empty(),
        "reach guard, got {effects:?}"
    );
    let Effect::DestroyAll { target, .. } = effects[1] else {
        panic!("expected DestroyAll, got {effects:?}");
    };
    assert_eq!(declared_slots(target), vec![0]);
}

/// SHAPE: Fiery Annihilation's Equipment target is narrowed to slot 0's
/// attachments, and the death-exile rider is bound to the creature's slot, not
/// to the Equipment node that precedes it (CR 614.1a).
#[test]
fn fiery_annihilation_narrows_equipment_slot_and_binds_rider_to_creature_slot() {
    let parsed = parse_oracle_text(
        FIERY_ANNIHILATION,
        "Fiery Annihilation",
        &[],
        &types("Instant"),
        &[],
    );
    let effects = chain(&parsed.abilities[0]);
    assert!(
        unimplemented_names(&[&parsed.abilities[0]]).is_empty(),
        "reach guard, got {effects:?}"
    );
    let Effect::ChangeZone { target, .. } = effects[1] else {
        panic!("expected the Equipment exile, got {effects:?}");
    };
    assert_eq!(declared_slots(target), vec![0]);
    let Effect::AddTargetReplacement { target, .. } = effects[2] else {
        panic!("expected the death-exile rider, got {effects:?}");
    };
    assert_eq!(*target, TargetFilter::ParentTargetSlot { index: 0 });
}

/// Out of class: "that creature" names no earlier declared target slot (a
/// trigger's other combatant; a granted trigger's own source). The fail-closed
/// gap stays. Paired with a reach guard that the clause was reached.
#[test]
fn referents_without_a_declared_slot_keep_their_gap() {
    for (oracle, name, core, gap) in [
        (
            TREEFOLK_MYSTIC,
            "Treefolk Mystic",
            "Creature",
            "attached_to_qualifier",
        ),
        (
            CORROSIVE_OOZE,
            "Corrosive Ooze",
            "Creature",
            "attached_to_qualifier",
        ),
        (
            SHACKLES_OF_TREACHERY,
            "Shackles of Treachery",
            "Sorcery",
            "unparsed_verb_arguments",
        ),
    ] {
        let parsed = parse_oracle_text(oracle, name, &[], &types(core), &[]);
        let json = serde_json::to_string(&parsed).expect("serialize parse");
        assert!(
            json.contains(gap),
            "{name}: the attachment clause must keep its `{gap}` gap: {json}"
        );
        assert!(
            !json.contains("DeclaredTarget"),
            "{name}: no declared-slot referent may be invented: {json}"
        );
    }
}

/// Two earlier creature targets make "that creature" ambiguous: no
/// nearest-declarer rule, the gap stays. Reach guard: the unambiguous control
/// with distinct nouns binds.
#[test]
fn ambiguous_antecedent_keeps_gap_and_distinct_nouns_bind() {
    let ambiguous = "Tap target creature. ~ deals 1 damage to target creature. Destroy all Equipment attached to that creature.";
    let parsed = parse_oracle_text(ambiguous, "Probe", &[], &types("Sorcery"), &[]);
    let json = serde_json::to_string(&parsed).unwrap();
    assert!(json.contains("attached_to_qualifier"), "{json}");
    assert!(!json.contains("DeclaredTarget"), "{json}");

    let distinct = "Tap target artifact. ~ deals 1 damage to target creature. Destroy all Equipment attached to that creature.";
    let parsed = parse_oracle_text(distinct, "Probe", &[], &types("Sorcery"), &[]);
    let effects = chain(&parsed.abilities[0]);
    let Some(Effect::DestroyAll { target, .. }) = effects.last().copied() else {
        panic!("expected DestroyAll, got {effects:?}");
    };
    assert_eq!(declared_slots(target), vec![1], "slot 1, not the artifact");
}

/// CR 601.2c: a player slot counts in the declared numbering, so the creature
/// is slot 1 (C1, mixed player + object chain).
#[test]
fn mixed_player_and_object_chain_numbers_the_creature_slot_one() {
    let text = "Target player loses 1 life. ~ deals 2 damage to target creature. Destroy all Equipment attached to that creature.";
    let parsed = parse_oracle_text(text, "Probe", &[], &types("Sorcery"), &[]);
    let effects = chain(&parsed.abilities[0]);
    let Some(Effect::DestroyAll { target, .. }) = effects.last().copied() else {
        panic!("expected DestroyAll, got {effects:?}");
    };
    assert_eq!(declared_slots(target), vec![1]);
}

/// H3: a "Choose target X and target Y" head's LOCAL slot index is never
/// trusted; the referent is re-resolved through the chain-wide registry. A
/// variable-count prefix keeps the gap.
#[test]
fn two_target_head_after_a_fixed_prefix_uses_the_chain_wide_slot() {
    let fixed = "Tap target land. Choose target creature and target artifact. Destroy all Equipment attached to that creature.";
    let parsed = parse_oracle_text(fixed, "Probe", &[], &types("Sorcery"), &[]);
    let effects = chain(&parsed.abilities[0]);
    let Some(Effect::DestroyAll { target, .. }) = effects.last().copied() else {
        panic!("expected DestroyAll, got {effects:?}");
    };
    assert_eq!(
        declared_slots(target),
        vec![1],
        "the creature is chain slot 1"
    );

    let variable = "Tap up to one target land. Choose target creature and target artifact. Destroy all Equipment attached to that creature.";
    let parsed = parse_oracle_text(variable, "Probe", &[], &types("Sorcery"), &[]);
    let json = serde_json::to_string(&parsed).unwrap();
    assert!(json.contains("attached_to_qualifier"), "{json}");
    assert!(!json.contains("DeclaredTarget"), "{json}");
}

/// CR 700.2: declared-slot numbering is mode-local, so a mode never admits the
/// referent. Reach guard: the same body outside a mode binds.
#[test]
fn modal_mode_declines_the_declared_slot_referent() {
    let body = "Target creature gets -1/-1 until end of turn. Destroy all Equipment attached to that creature.";
    let parsed = parse_oracle_text(body, "Probe", &[], &types("Sorcery"), &[]);
    let json = serde_json::to_string(&parsed).unwrap();
    assert!(json.contains("DeclaredTarget"), "control binds: {json}");

    let modal = format!("Choose one —\n• Draw a card.\n• {body}");
    let parsed = parse_oracle_text(&modal, "Probe", &[], &types("Sorcery"), &[]);
    let json = serde_json::to_string(&parsed).unwrap();
    assert!(!json.contains("DeclaredTarget"), "{json}");
    assert!(json.contains("attached_to_qualifier"), "{json}");
}

// ---------------------------------------------------------------------------
// Runtime rows (cast pipeline)
// ---------------------------------------------------------------------------

fn equipment(
    scenario: &mut GameScenario,
    owner: engine::types::player::PlayerId,
    name: &str,
) -> ObjectId {
    scenario
        .add_artifact_from_oracle(owner, name, "Equipped creature gets +1/+0.")
        .with_subtypes(vec!["Equipment"])
        .id()
}

fn aura(
    scenario: &mut GameScenario,
    owner: engine::types::player::PlayerId,
    name: &str,
) -> ObjectId {
    scenario
        .add_enchantment_from_oracle(
            owner,
            name,
            "Enchant creature\nEnchanted creature gets +1/+1.",
        )
        .with_subtypes(vec!["Aura"])
        .id()
}

fn free_spell(scenario: &mut GameScenario, name: &str, instant: bool, oracle: &str) -> ObjectId {
    scenario
        .add_spell_to_hand_from_oracle(P0, name, instant, oracle)
        .with_mana_cost(ManaCost::generic(0))
        .id()
}

fn eligible(waiting: &WaitingFor) -> (Vec<ObjectId>, u32, Option<u32>) {
    let WaitingFor::ChooseObjectsSelection {
        eligible, min, max, ..
    } = waiting
    else {
        panic!("expected the resolution-time Equipment choice, got {waiting:?}");
    };
    let mut ids: Vec<ObjectId> = eligible
        .iter()
        .filter_map(|t| match t {
            TargetRef::Object(id) => Some(*id),
            TargetRef::Player(_) => None,
        })
        .collect();
    ids.sort();
    (ids, *min, *max)
}

fn choose(runner: &mut GameRunner, chosen: &[ObjectId]) {
    runner
        .act(GameAction::SelectTargets {
            targets: chosen.iter().map(|id| TargetRef::Object(*id)).collect(),
        })
        .expect("the selection must be accepted");
    runner.advance_until_stack_empty();
}

fn give_hexproof(state: &mut engine::types::game_state::GameState, id: ObjectId) {
    let obj = state.objects.get_mut(&id).unwrap();
    obj.base_keywords.push(Keyword::Hexproof);
    obj.keywords.push(Keyword::Hexproof);
}

struct LojBoard {
    runner: GameRunner,
    spell: ObjectId,
    victim: ObjectId,
    other: ObjectId,
    eq_a: ObjectId,
    aura_b: ObjectId,
    eq_c: ObjectId,
    eq_d: ObjectId,
}

/// The opponent's 2/7 `victim` carries Equipment A and Aura B; Equipment C is on
/// another creature; Equipment D is unattached.
fn loj_board(oracle: &str, name: &str, victim_toughness: i32) -> LojBoard {
    let mut scenario = GameScenario::new();
    scenario.at_phase(Phase::PreCombatMain);
    let victim = scenario
        .add_creature(P1, "Victim", 2, victim_toughness)
        .id();
    let other = scenario.add_creature(P1, "Other", 2, 7).id();
    let eq_a = equipment(&mut scenario, P1, "Equipment A");
    let aura_b = aura(&mut scenario, P1, "Aura B");
    let eq_c = equipment(&mut scenario, P1, "Equipment C");
    let eq_d = equipment(&mut scenario, P1, "Equipment D");
    let spell = free_spell(&mut scenario, name, true, oracle);
    let mut runner = scenario.build();
    attach::attach_to(runner.state_mut(), eq_a, victim);
    attach::attach_to(runner.state_mut(), aura_b, victim);
    attach::attach_to(runner.state_mut(), eq_c, other);
    LojBoard {
        runner,
        spell,
        victim,
        other,
        eq_a,
        aura_b,
        eq_c,
        eq_d,
    }
}

/// R1 (CR 115.10a + CR 608.2d): the choice is made while the spell resolves and
/// offers exactly the Equipment attached to the targeted creature ? not its
/// Aura, not Equipment on another creature, not unattached Equipment.
#[test]
fn light_of_judgment_offers_only_the_targets_equipment_and_destroys_the_choice() {
    let mut b = loj_board(LIGHT_OF_JUDGMENT, "Light of Judgment", 7);
    let outcome = b.runner.cast(b.spell).target_objects(&[b.victim]).resolve();
    let (ids, min, max) = eligible(outcome.final_waiting_for());
    assert_eq!(ids, vec![b.eq_a], "only the target's Equipment is offered");
    assert_eq!((min, max), (0, Some(1)), "up to one");
    assert_eq!(
        outcome.state().objects[&b.victim].damage_marked,
        6,
        "reach guard: the first sentence resolved before the choice (CR 608.2c)"
    );
    choose(&mut b.runner, &[b.eq_a]);
    let state = b.runner.state();
    assert_eq!(state.objects[&b.eq_a].zone, Zone::Graveyard);
    for survivor in [b.aura_b, b.eq_c, b.eq_d] {
        assert_eq!(state.objects[&survivor].zone, Zone::Battlefield);
    }
}

/// R2: "up to one" admits choosing nothing; and a target with no Equipment
/// still raises the (empty) choice, a legal zero selection.
#[test]
fn light_of_judgment_choosing_none_destroys_nothing_and_unequipped_target_is_legal() {
    let mut b = loj_board(LIGHT_OF_JUDGMENT, "Light of Judgment", 7);
    let outcome = b.runner.cast(b.spell).target_objects(&[b.victim]).resolve();
    let (ids, _, _) = eligible(outcome.final_waiting_for());
    assert_eq!(ids, vec![b.eq_a], "reach guard");
    choose(&mut b.runner, &[]);
    assert_eq!(b.runner.state().objects[&b.eq_a].zone, Zone::Battlefield);
    assert!(
        b.runner.state().stack.is_empty(),
        "the spell finished resolving"
    );

    // Unequipped target: the choice is offered with nothing eligible.
    let mut b = loj_board(LIGHT_OF_JUDGMENT, "Light of Judgment", 7);
    let outcome = b.runner.cast(b.spell).target_objects(&[b.other]).resolve();
    let (ids, _, max) = eligible(outcome.final_waiting_for());
    assert_eq!(ids, vec![b.eq_c], "Other carries Equipment C");
    assert_eq!(max, Some(1));
    let mut b = loj_board(LIGHT_OF_JUDGMENT, "Light of Judgment", 7);
    attach::attach_to(b.runner.state_mut(), b.eq_a, b.other);
    let outcome = b.runner.cast(b.spell).target_objects(&[b.victim]).resolve();
    let (ids, _, max) = eligible(outcome.final_waiting_for());
    assert!(ids.is_empty(), "an unequipped target offers nothing");
    assert_eq!(max, Some(0), "CR 609.3: the achievable maximum is zero");
    choose(&mut b.runner, &[]);
    assert_eq!(b.runner.state().objects[&b.victim].damage_marked, 6);
}

/// R3 (CR 608.2d, ruling 1): the Equipment is chosen while the spell resolves,
/// so an Equipment moved away in response is not offered and one attached in
/// response is.
#[test]
fn light_of_judgment_reads_attachments_at_resolution_not_at_cast() {
    let mut b = loj_board(LIGHT_OF_JUDGMENT, "Light of Judgment", 7);
    let mut commit = b.runner.cast(b.spell).target_objects(&[b.victim]).commit();
    attach::attach_to(commit.state_mut(), b.eq_a, b.other);
    attach::attach_to(commit.state_mut(), b.eq_d, b.victim);
    let outcome = commit.resolve();
    let (ids, _, _) = eligible(outcome.final_waiting_for());
    assert_eq!(
        ids,
        vec![b.eq_d],
        "the response-attached Equipment D, not A"
    );
}

/// R4 (CR 608.2b, ruling 3): the creature becomes an illegal target in
/// response, so the spell does not resolve and no Equipment is destroyed.
#[test]
fn light_of_judgment_illegal_target_destroys_nothing() {
    let mut b = loj_board(LIGHT_OF_JUDGMENT, "Light of Judgment", 7);
    let mut commit = b.runner.cast(b.spell).target_objects(&[b.victim]).commit();
    give_hexproof(commit.state_mut(), b.victim);
    let outcome = commit.resolve();
    assert!(
        matches!(outcome.final_waiting_for(), WaitingFor::Priority { .. }),
        "no choice is raised, got {:?}",
        outcome.final_waiting_for()
    );
    assert_eq!(outcome.state().objects[&b.victim].damage_marked, 0);
    assert_eq!(outcome.zone_of(b.eq_a), Zone::Battlefield);
    assert_eq!(
        outcome.zone_of(b.spell),
        Zone::Graveyard,
        "reach guard: it was cast"
    );
}

/// R5 (CR 704.3): lethal damage does not remove the creature until after the
/// spell resolves, so its Equipment is still offered and destroyed.
#[test]
fn light_of_judgment_lethal_damage_still_offers_the_equipment() {
    let mut b = loj_board(LIGHT_OF_JUDGMENT, "Light of Judgment", 2);
    let outcome = b.runner.cast(b.spell).target_objects(&[b.victim]).resolve();
    let (ids, _, _) = eligible(outcome.final_waiting_for());
    assert_eq!(ids, vec![b.eq_a]);
    choose(&mut b.runner, &[b.eq_a]);
    assert_eq!(b.runner.state().objects[&b.eq_a].zone, Zone::Graveyard);
    assert_eq!(
        b.runner.state().objects[&b.victim].zone,
        Zone::Graveyard,
        "the creature dies to state-based actions after resolution"
    );
}

/// R6 (CR 701.8b): indestructible Equipment can be chosen but is not destroyed.
/// R7: the Equipment's controller is irrelevant ? the caster's own Equipment on
/// the opponent's creature is offered too.
#[test]
fn light_of_judgment_indestructible_and_foreign_controller_equipment() {
    let mut scenario = GameScenario::new();
    scenario.at_phase(Phase::PreCombatMain);
    let victim = scenario.add_creature(P1, "Victim", 2, 7).id();
    let sturdy = scenario
        .add_artifact_from_oracle(P1, "Sturdy", "Equipped creature gets +1/+0.")
        .with_subtypes(vec!["Equipment"])
        .indestructible()
        .id();
    let mine = equipment(&mut scenario, P0, "Mine");
    let spell = free_spell(&mut scenario, "Light of Judgment", true, LIGHT_OF_JUDGMENT);
    let mut runner = scenario.build();
    attach::attach_to(runner.state_mut(), sturdy, victim);
    attach::attach_to(runner.state_mut(), mine, victim);
    let outcome = runner.cast(spell).target_objects(&[victim]).resolve();
    let (ids, _, _) = eligible(outcome.final_waiting_for());
    let mut expected = vec![sturdy, mine];
    expected.sort();
    assert_eq!(
        ids, expected,
        "both Equipment are offered, whoever controls them"
    );
    choose(&mut runner, &[sturdy]);
    assert_eq!(
        runner.state().objects[&sturdy].zone,
        Zone::Battlefield,
        "indestructible Equipment survives (CR 701.8b)"
    );
    assert_eq!(runner.state().objects[&mine].zone, Zone::Battlefield);
}

/// Turn to Slag: every Equipment on the target is destroyed; Equipment on
/// another creature and the target's Aura survive.
#[test]
fn turn_to_slag_destroys_all_equipment_on_the_target_only() {
    let mut b = loj_board(TURN_TO_SLAG, "Turn to Slag", 7);
    let second = b.eq_d;
    attach::attach_to(b.runner.state_mut(), second, b.victim);
    let outcome = b.runner.cast(b.spell).target_objects(&[b.victim]).resolve();
    assert!(matches!(
        outcome.final_waiting_for(),
        WaitingFor::Priority { .. }
    ));
    assert_eq!(outcome.zone_of(b.eq_a), Zone::Graveyard);
    assert_eq!(outcome.zone_of(second), Zone::Graveyard);
    assert_eq!(
        outcome.zone_of(b.eq_c),
        Zone::Battlefield,
        "other creature's Equipment"
    );
    assert_eq!(
        outcome.zone_of(b.aura_b),
        Zone::Battlefield,
        "an Aura is not Equipment"
    );
    assert_eq!(outcome.state().objects[&b.victim].damage_marked, 5);
}

/// T9 (CR 608.2h): the referent was moved by a LEGAL earlier instruction of the
/// same resolution, so "Equipment attached to that creature" reads the exact
/// exit record of that creature ? its former Equipment is destroyed, Equipment
/// elsewhere is not.
#[test]
fn referent_moved_by_an_earlier_instruction_reads_its_exit_record() {
    const TEXT: &str = "Exile target creature. Destroy all Equipment attached to that creature.";
    let mut b = loj_board(TEXT, "Probe", 7);
    let outcome = b.runner.cast(b.spell).target_objects(&[b.victim]).resolve();
    assert_eq!(
        outcome.zone_of(b.victim),
        Zone::Exile,
        "reach guard: exiled first"
    );
    assert_eq!(
        outcome.zone_of(b.eq_a),
        Zone::Graveyard,
        "the exiled creature's former Equipment is destroyed (CR 608.2h)"
    );
    assert_eq!(outcome.zone_of(b.eq_c), Zone::Battlefield);
    assert_eq!(outcome.zone_of(b.eq_d), Zone::Battlefield);
}

/// C1 (CR 601.2c): the creature is declared slot 1 behind a player slot; the
/// referent reads slot 1. Control: the player slot stays legal while the
/// creature becomes illegal ? the spell resolves, but the Equipment part needs
/// information about the illegal creature and does nothing (CR 608.2b).
#[test]
fn mixed_player_object_chain_reads_slot_one_and_illegal_later_slot_supplies_nothing() {
    const TEXT: &str = "Target player loses 1 life. ~ deals 2 damage to target creature. Destroy all Equipment attached to that creature.";
    let mut b = loj_board(TEXT, "Probe", 7);
    let outcome = b
        .runner
        .cast(b.spell)
        .target_player(P1)
        .target_objects(&[b.victim])
        .resolve();
    outcome.assert_life_delta(P1, -1);
    assert_eq!(outcome.zone_of(b.eq_a), Zone::Graveyard);
    assert_eq!(outcome.zone_of(b.eq_c), Zone::Battlefield);

    let mut b = loj_board(TEXT, "Probe", 7);
    let mut commit = b
        .runner
        .cast(b.spell)
        .target_player(P1)
        .target_objects(&[b.victim])
        .commit();
    give_hexproof(commit.state_mut(), b.victim);
    let outcome = commit.resolve();
    outcome.assert_life_delta(P1, -1);
    assert_eq!(outcome.state().objects[&b.victim].damage_marked, 0);
    assert_eq!(
        outcome.zone_of(b.eq_a),
        Zone::Battlefield,
        "an illegal referent supplies no information (CR 608.2b)"
    );
}

/// C2 (CR 115.3): with distinct nouns, slot 0 (an artifact) and slot 1 (a
/// creature) may be the same object or different objects; the referent is
/// slot 1, never "the first object target".
#[test]
fn same_object_in_two_slots_and_split_slots_read_slot_one() {
    const TEXT: &str = "Tap target artifact. ~ deals 1 damage to target creature. Destroy all Equipment attached to that creature.";
    let build = || {
        let mut scenario = GameScenario::new();
        scenario.at_phase(Phase::PreCombatMain);
        let a = scenario
            .add_creature(P1, "Artifact Creature A", 2, 7)
            .as_artifact()
            .as_creature()
            .id();
        let bee = scenario.add_creature(P1, "Creature B", 2, 7).id();
        let eq_a = equipment(&mut scenario, P1, "On A");
        let eq_b = equipment(&mut scenario, P1, "On B");
        let spell = free_spell(&mut scenario, "Probe", false, TEXT);
        let mut runner = scenario.build();
        attach::attach_to(runner.state_mut(), eq_a, a);
        attach::attach_to(runner.state_mut(), eq_b, bee);
        (runner, spell, a, bee, eq_a, eq_b)
    };
    // (a) A fills both slots.
    let (mut runner, spell, a, _bee, eq_a, eq_b) = build();
    let outcome = runner.cast(spell).target_objects(&[a, a]).resolve();
    assert!(
        outcome.state().objects[&a].tapped,
        "reach guard: slot 0 is A"
    );
    assert_eq!(outcome.zone_of(eq_a), Zone::Graveyard);
    assert_eq!(outcome.zone_of(eq_b), Zone::Battlefield);
    // (b) A in slot 0, B in slot 1: B's Equipment, not A's.
    let (mut runner, spell, a, bee, eq_a, eq_b) = build();
    let outcome = runner.cast(spell).target_objects(&[a, bee]).resolve();
    assert!(
        outcome.state().objects[&a].tapped,
        "reach guard: slot 0 is A"
    );
    assert_eq!(outcome.zone_of(eq_b), Zone::Graveyard, "slot 1's Equipment");
    assert_eq!(
        outcome.zone_of(eq_a),
        Zone::Battlefield,
        "not the first object's"
    );
}

// ---------------------------------------------------------------------------
// Fiery Annihilation (whole card)
// ---------------------------------------------------------------------------

/// Move `id` from the battlefield to its owner's graveyard ("would die",
/// CR 700.4) through the zone pipeline, so replacement effects apply.
fn kill(runner: &mut GameRunner, id: ObjectId) {
    let mut events: Vec<GameEvent> = Vec::new();
    move_object_for_test(
        runner.state_mut(),
        ZoneMoveRequest::effect(id, Zone::Graveyard, id),
        &mut events,
    );
}

/// Pass priority (declaring no attackers or blockers) until `turn` begins.
fn drive_to_turn(runner: &mut GameRunner, turn: u32) {
    for _ in 0..300 {
        if runner.state().turn_number >= turn {
            return;
        }
        let action = match &runner.state().waiting_for {
            WaitingFor::Priority { .. } => GameAction::PassPriority,
            WaitingFor::DeclareAttackers { .. } => GameAction::DeclareAttackers {
                attacks: Vec::<(ObjectId, AttackTarget)>::new(),
                bands: vec![],
            },
            WaitingFor::DeclareBlockers { .. } => GameAction::DeclareBlockers {
                assignments: Vec::new(),
            },
            other => panic!("unexpected prompt while driving: {other:?}"),
        };
        runner.act(action).expect("driver action accepted");
    }
    panic!("turn {turn} never began");
}

/// F7 (CR 601.2c): once the creature is chosen, the Equipment slot offers only
/// Equipment attached to THAT creature ? not the caster's own unattached
/// Equipment, not Equipment on a different creature.
#[test]
fn fiery_annihilation_equipment_slot_offers_only_the_chosen_creatures_equipment() {
    let mut b = loj_board(FIERY_ANNIHILATION, "Fiery Annihilation", 7);
    let mine = b.eq_d;
    // The caster's own unattached Equipment.
    {
        let state = b.runner.state_mut();
        let obj = state.objects.get_mut(&mine).unwrap();
        obj.controller = P0;
        obj.owner = P0;
    }
    let card_id = b.runner.state().objects[&b.spell].card_id;
    b.runner
        .act(GameAction::CastSpell {
            object_id: b.spell,
            card_id,
            targets: vec![],
            payment_mode: engine::types::game_state::CastPaymentMode::Auto,
        })
        .expect("cast");
    b.runner
        .act(GameAction::ChooseTarget {
            target: Some(TargetRef::Object(b.victim)),
        })
        .expect("choose the creature");
    let WaitingFor::TargetSelection {
        target_slots,
        selection,
        ..
    } = &b.runner.state().waiting_for
    else {
        panic!(
            "expected the Equipment slot, got {:?}",
            b.runner.state().waiting_for
        );
    };
    let offered = &selection.current_legal_targets;
    assert!(
        target_slots[selection.current_slot]
            .legal_targets
            .contains(&TargetRef::Object(b.eq_a)),
        "reach guard: the static slot admits the Equipment"
    );
    assert_eq!(
        offered,
        &vec![TargetRef::Object(b.eq_a)],
        "only the chosen creature's Equipment"
    );
    assert!(!offered.contains(&TargetRef::Object(mine)));
    assert!(!offered.contains(&TargetRef::Object(b.eq_c)));
    assert!(
        b.runner
            .act(GameAction::ChooseTarget {
                target: Some(TargetRef::Object(b.eq_c)),
            })
            .is_err(),
        "submitting another creature's Equipment is rejected"
    );
}

/// F1 + F5 + F2 (CR 614.1a): with the Equipment chosen and still attached, it
/// is exiled; lethal damage then exiles the CREATURE instead of putting it into
/// the graveyard ? the rider is bound to the creature's slot, not the
/// Equipment node it follows.
#[test]
fn fiery_annihilation_exiles_equipment_and_rider_exiles_the_creature() {
    let mut b = loj_board(FIERY_ANNIHILATION, "Fiery Annihilation", 2);
    let outcome = b
        .runner
        .cast(b.spell)
        .target_objects(&[b.victim, b.eq_a])
        .resolve();
    assert_eq!(
        outcome.zone_of(b.eq_a),
        Zone::Exile,
        "the Equipment is exiled"
    );
    assert_eq!(
        outcome.zone_of(b.victim),
        Zone::Exile,
        "the creature would die to lethal damage and is exiled instead"
    );
    assert_eq!(outcome.zone_of(b.eq_c), Zone::Battlefield);

    // F1: no Equipment chosen ? damage and the rider still apply.
    let mut b = loj_board(FIERY_ANNIHILATION, "Fiery Annihilation", 2);
    let outcome = b.runner.cast(b.spell).target_objects(&[b.victim]).resolve();
    assert_eq!(outcome.zone_of(b.victim), Zone::Exile);
    assert_eq!(
        outcome.zone_of(b.eq_a),
        Zone::Battlefield,
        "not chosen, not exiled"
    );
}

/// F3 (CR 608.2b, ruling 3): the Equipment moved to another creature in
/// response is no longer attached to the target, so it is an illegal target and
/// is not exiled; the creature is still dealt damage and the rider still
/// applies to it.
#[test]
fn fiery_annihilation_moved_equipment_is_not_exiled_but_damage_and_rider_apply() {
    let mut b = loj_board(FIERY_ANNIHILATION, "Fiery Annihilation", 2);
    let mut commit = b
        .runner
        .cast(b.spell)
        .target_objects(&[b.victim, b.eq_a])
        .commit();
    attach::attach_to(commit.state_mut(), b.eq_a, b.other);
    let outcome = commit.resolve();
    assert_eq!(outcome.zone_of(b.eq_a), Zone::Battlefield);
    assert_eq!(
        outcome.state().objects[&b.eq_a].attached_to,
        Some(AttachTarget::Object(b.other))
    );
    assert_eq!(
        outcome.zone_of(b.victim),
        Zone::Exile,
        "damage + rider applied"
    );
}

/// F4 (CR 608.2b, ruling 2): the creature becomes an illegal target while the
/// Equipment is otherwise selectable. The Equipment part needs information
/// about the illegal creature, so nothing is exiled, no damage is dealt, and no
/// rider is installed.
#[test]
fn fiery_annihilation_illegal_creature_exiles_nothing_and_installs_no_rider() {
    let mut b = loj_board(FIERY_ANNIHILATION, "Fiery Annihilation", 7);
    let mut commit = b
        .runner
        .cast(b.spell)
        .target_objects(&[b.victim, b.eq_a])
        .commit();
    give_hexproof(commit.state_mut(), b.victim);
    let outcome = commit.resolve();
    assert_eq!(outcome.state().objects[&b.victim].damage_marked, 0);
    assert_eq!(outcome.zone_of(b.eq_a), Zone::Battlefield);
    assert_eq!(
        outcome.zone_of(b.spell),
        Zone::Graveyard,
        "reach guard: cast"
    );
    kill(&mut b.runner, b.victim);
    assert_eq!(
        b.runner.state().objects[&b.victim].zone,
        Zone::Graveyard,
        "no rider was installed"
    );
}

/// F6 (ruling 4): the rider applies if the creature would die this turn FOR
/// ANY REASON ? a later, independent destruction exiles it; the Equipment
/// stays. F8: after the turn ends, the "this turn" rider has expired.
#[test]
fn fiery_annihilation_rider_applies_to_any_death_this_turn_and_expires() {
    let mut b = loj_board(FIERY_ANNIHILATION, "Fiery Annihilation", 7);
    let outcome = b.runner.cast(b.spell).target_objects(&[b.victim]).resolve();
    assert_eq!(outcome.state().objects[&b.victim].damage_marked, 5);
    kill(&mut b.runner, b.victim);
    assert_eq!(b.runner.state().objects[&b.victim].zone, Zone::Exile);
    assert_eq!(b.runner.state().objects[&b.eq_a].zone, Zone::Battlefield);

    let mut b = loj_board(FIERY_ANNIHILATION, "Fiery Annihilation", 7);
    let outcome = b.runner.cast(b.spell).target_objects(&[b.victim]).resolve();
    assert_eq!(outcome.state().objects[&b.victim].damage_marked, 5);
    let next = b.runner.state().turn_number + 1;
    drive_to_turn(&mut b.runner, next);
    kill(&mut b.runner, b.victim);
    assert_eq!(
        b.runner.state().objects[&b.victim].zone,
        Zone::Graveyard,
        "the rider expired with the turn"
    );
}
