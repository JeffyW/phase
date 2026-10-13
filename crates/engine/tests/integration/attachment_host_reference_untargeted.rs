//! "Enchanted creature" / "equipped creature" in an Aura's or Equipment's own
//! ability names its host; it is not a target (phase-rs/phase#9824).
//!
//! CR 115.10a: an object affected by a spell or ability is a target only if the
//! text identifies it with the word "target". CR 303.4b / CR 301.5a + CR 301.5f:
//! "enchanted"/"equipped" names whatever the source is attached to. CR 115.1b: an
//! Aura spell is targeted, but an Aura permanent targets nothing. So such an
//! instruction never makes its host "become the target" (no ward trigger, CR
//! 702.21a), is not stopped by shroud/hexproof (CR 702.18a / CR 702.11b), and
//! reads its host when it resolves, using the source's last known information
//! when the source has left the battlefield (CR 608.2h).
//!
//! Oracle text below is verbatim from Scryfall `cards/named`.

use engine::game::game_object::AttachTarget;
use engine::game::scenario::{GameRunner, GameScenario, P0, P1};
use engine::types::ability::{Effect, TargetChoiceTiming, TargetRef};
use engine::types::actions::{AlternativeCastDecision, GameAction};
use engine::types::game_state::{CastPaymentMode, StackEntryKind, WaitingFor};
use engine::types::identifiers::ObjectId;
use engine::types::keywords::{Keyword, WardCost};
use engine::types::mana::{ManaCost, ManaType, ManaUnit};
use engine::types::phase::Phase;
use engine::types::player::PlayerId;
use engine::types::zones::Zone;

const MALFUNCTION: &str = "Enchant artifact or creature\nWhen this Aura enters, tap enchanted permanent.\nEnchanted permanent doesn't untap during its controller's untap step.";
const CLAUSTROPHOBIA: &str = "Enchant creature\nWhen this Aura enters, tap enchanted creature.\nEnchanted creature doesn't untap during its controller's untap step.";
const BIND_THE_MONSTER: &str = "Enchant creature\nWhen this Aura enters, tap enchanted creature. It deals damage to you equal to its power.\nEnchanted creature doesn't untap during its controller's untap step.";
const FATAL_ATTRACTION: &str = "Enchant creature\nWhen this Aura enters, it deals 2 damage to enchanted creature.\nAt the beginning of your upkeep, this Aura deals 4 damage to enchanted creature.";
const PEMMINS_AURA: &str = "Enchant creature\n{U}: Untap enchanted creature.\n{U}: Enchanted creature gains flying until end of turn.\n{U}: Enchanted creature gains shroud until end of turn. (It can't be the target of spells or abilities.)\n{1}: Enchanted creature gets +1/-1 or -1/+1 until end of turn.";
const CLOAK_OF_THE_PHILOSOPHER: &str =
    "Equipped creature gets +1/+1.\n{2}: Untap equipped creature.\nEquip {2}";
const PARALLAX_DEMENTIA_LEAVES: &str = "Enchant creature\nEnchanted creature gets +3/+2.\nWhen this Aura leaves the battlefield, destroy enchanted creature. That creature can't be regenerated.";
const NURTURING_LICID: &str = "{G}, {T}: This creature loses this ability and becomes an Aura enchantment with enchant creature. Attach it to target creature. You may pay {G} to end this effect.\n{G}: Regenerate enchanted creature.";
const FRENZIED_FUGUE: &str = "Enchant permanent\nWhen this Aura enters and at the beginning of your upkeep, gain control of enchanted permanent until end of turn. Untap that permanent. It gains haste until end of turn.";
const MAROONED: &str = "Enchant artifact or creature\nWhen this Aura enters, you may pay {3}. If you do, tap enchanted permanent.\nEnchanted permanent doesn't untap during its controller's untap step and can't be beamed up.";
const SWORD_OF_THE_PARUNS: &str = "As long as equipped creature is tapped, tapped creatures you control get +2/+0.\nAs long as equipped creature is untapped, untapped creatures you control get +0/+2.\n{3}: You may tap or untap equipped creature.\nEquip {3}";
const SUN_CLASP: &str =
    "Enchant creature\nEnchanted creature gets +1/+3.\n{W}: Return enchanted creature to its owner's hand.";
const VANISHING: &str = "Enchant creature\n{U}{U}: Enchanted creature phases out. (While it's phased out, it's treated as though it doesn't exist. It phases in before its controller untaps during their next untap step.)";
const SHATTERED_EGO: &str = "Enchant creature\nEnchanted creature gets -3/-0.\n{3}{U}{U}: Put enchanted creature into its owner's library third from the top.";
const JUNK_JET: &str = "When this Equipment enters, create a Junk token. (It's an artifact with \"{T}, Sacrifice this token: Exile the top card of your library. You may play that card this turn. Activate only as a sorcery.\")\n{3}, Sacrifice another artifact: Double equipped creature's power until end of turn.\nEquip {1}";
const FOLLOWED_FOOTSTEPS: &str = "Enchant creature\nAt the beginning of your upkeep, create a token that's a copy of enchanted creature.";
const CONVENIENT_TARGET: &str = "Enchant creature\nWhen this Aura enters, suspect enchanted creature. (It has menace and can't block.)\nEnchanted creature gets +1/+1.\n{2}{R}: Return this card from your graveyard to your hand.";

fn free_cost() -> ManaCost {
    ManaCost::Cost {
        shards: vec![],
        generic: 0,
    }
}

fn pool(blue: usize, colorless: usize) -> Vec<ManaUnit> {
    let unit = |t| ManaUnit::new(t, ObjectId(0), false, vec![]);
    std::iter::repeat_n(unit(ManaType::Blue), blue)
        .chain(std::iter::repeat_n(unit(ManaType::Colorless), colorless))
        .collect()
}

fn aura_in_hand(sc: &mut GameScenario, player: PlayerId, name: &str, text: &str) -> ObjectId {
    sc.add_spell_to_hand(player, name, false)
        .as_enchantment()
        .with_subtypes(vec!["Aura"])
        .with_mana_cost(free_cost())
        .from_oracle_text_with_keywords(&["enchant"], text)
        .id()
}

fn aura_on_battlefield(
    sc: &mut GameScenario,
    player: PlayerId,
    name: &str,
    text: &str,
) -> ObjectId {
    sc.add_creature(player, name, 0, 0)
        .as_enchantment()
        .with_subtypes(vec!["Aura"])
        .from_oracle_text_with_keywords(&["enchant"], text)
        .id()
}

fn attach(runner: &mut GameRunner, attachment: ObjectId, host: ObjectId) {
    runner
        .state_mut()
        .objects
        .get_mut(&attachment)
        .unwrap()
        .attached_to = Some(AttachTarget::Object(host));
    let host_obj = runner.state_mut().objects.get_mut(&host).unwrap();
    if !host_obj.attachments.contains(&attachment) {
        host_obj.attachments.push(attachment);
    }
    runner.state_mut().layers_dirty.mark_full();
}

fn set_tapped(runner: &mut GameRunner, id: ObjectId, tapped: bool) {
    runner.state_mut().objects.get_mut(&id).unwrap().tapped = tapped;
}

fn ward_triggers_on_stack(runner: &GameRunner, ward_holder: ObjectId) -> usize {
    runner
        .state()
        .stack
        .iter()
        .filter(|e| {
            matches!(&e.kind, StackEntryKind::TriggeredAbility { source_id, .. } if *source_id == ward_holder)
        })
        .count()
}

/// The index of `source`'s first activated ability whose effect matches `pick`.
fn ability_index(runner: &GameRunner, source: ObjectId, pick: fn(&Effect) -> bool) -> usize {
    runner.state().objects[&source]
        .abilities
        .iter()
        .position(|a| pick(&a.effect))
        .expect("the ability is present")
}

#[derive(Debug, Default)]
struct Drive {
    targeted_aura_spell: bool,
    ward_after_cast: usize,
    /// Every distinct ward trigger the host put on the stack during the drive.
    ward_triggers: std::collections::HashSet<ObjectId>,
}

impl Drive {
    fn ward_triggers(&self) -> usize {
        self.ward_triggers.len()
    }
}

/// Cast the Aura spell at `host`, then resolve everything. Pays the first ward
/// trigger's cost and declines a second one (the live game had no mana left);
/// accepts every "you may".
fn cast_aura_and_resolve(
    runner: &mut GameRunner,
    aura: ObjectId,
    host: ObjectId,
    mode: CastPaymentMode,
) -> Drive {
    let mut d = Drive::default();
    let card_id = runner.state().objects[&aura].card_id;
    runner
        .act(GameAction::CastSpell {
            object_id: aura,
            card_id,
            targets: vec![],
            payment_mode: mode,
        })
        .expect("the Aura spell is announced");
    let mut announcing = true;
    for _ in 0..64 {
        let wf = runner.state().waiting_for.clone();
        d.ward_triggers.extend(runner.state().stack.iter().filter_map(|e| {
            matches!(&e.kind, StackEntryKind::TriggeredAbility { source_id, .. } if *source_id == host)
                .then_some(e.id)
        }));
        if announcing && matches!(wf, WaitingFor::Priority { .. }) {
            announcing = false;
            d.ward_after_cast = ward_triggers_on_stack(runner, host);
        }
        if !announcing
            && runner.state().stack.is_empty()
            && matches!(wf, WaitingFor::Priority { .. })
        {
            return d;
        }
        let action = match wf {
            WaitingFor::Priority { .. } => GameAction::PassPriority,
            WaitingFor::AlternativeCastChoice { .. } => GameAction::ChooseAlternativeCast {
                choice: AlternativeCastDecision::Normal,
            },
            WaitingFor::TargetSelection { .. } | WaitingFor::TriggerTargetSelection { .. } => {
                d.targeted_aura_spell |= announcing;
                GameAction::SelectTargets {
                    targets: vec![TargetRef::Object(host)],
                }
            }
            WaitingFor::ManaPayment { .. } => GameAction::PassPriority,
            WaitingFor::UnlessPayment { .. } => GameAction::PayUnlessCost {
                pay: d.ward_triggers() < 2,
            },
            WaitingFor::OptionalEffectChoice { .. } => {
                GameAction::DecideOptionalEffect { accept: true }
            }
            WaitingFor::OrderTriggers { triggers, .. } => GameAction::OrderTriggers {
                order: (0..triggers.len()).collect(),
            },
            other => panic!("unexpected prompt: {other:?}"),
        };
        runner.act(action).expect("prompt accepts");
    }
    panic!("the stack did not resolve");
}

/// P1 controls a Ward {2} creature; P0 casts `aura_text` at it with exactly
/// enough mana for ONE ward payment.
fn ward_host_case(aura_name: &str, aura_text: &str, mode: CastPaymentMode) {
    let mut sc = GameScenario::new();
    sc.at_phase(Phase::PreCombatMain);
    let host = sc
        .add_creature(P1, "Patchwork Automaton", 1, 1)
        .with_keyword(Keyword::Ward(WardCost::Mana(ManaCost::generic(2))))
        .id();
    let aura = aura_in_hand(&mut sc, P0, aura_name, aura_text);
    // A second legal enchant target, so the Aura spell's target is a real choice.
    sc.add_creature(P0, "Grizzly Bears", 2, 2);
    sc.with_mana_pool(P0, pool(0, 2));
    let mut runner = sc.build();

    let d = cast_aura_and_resolve(&mut runner, aura, host, mode);
    let s = runner.state();

    assert!(
        d.targeted_aura_spell,
        "reach ({aura_name}, {mode:?}): the Aura spell chose its enchant target"
    );
    // CR 702.21a + CR 115.1b: the Aura spell's one targeting triggers ward once.
    assert_eq!(
        d.ward_after_cast, 1,
        "({aura_name}, {mode:?}) the Aura spell's targeting triggers ward once"
    );
    assert_eq!(
        s.objects[&aura].attached_to,
        Some(AttachTarget::Object(host)),
        "reach ({aura_name}, {mode:?}): ward was paid, so the Aura resolved and attached"
    );
    // CR 115.10a: "tap enchanted permanent/creature" has no "target", so the ETB
    // never makes the host become the target and ward asks exactly once.
    assert_eq!(
        d.ward_triggers(),
        1,
        "({aura_name}, {mode:?}) ward triggers exactly once"
    );
    assert!(
        s.objects[&host].tapped,
        "({aura_name}, {mode:?}) the ETB taps the enchanted permanent"
    );
}

#[test]
fn malfunction_ward_asks_once_and_etb_taps_host_auto_payment() {
    ward_host_case("Malfunction", MALFUNCTION, CastPaymentMode::Auto);
}

#[test]
fn malfunction_ward_asks_once_and_etb_taps_host_interactive_payment() {
    ward_host_case("Malfunction", MALFUNCTION, CastPaymentMode::Manual);
}

#[test]
fn claustrophobia_ward_asks_once_and_etb_taps_host() {
    ward_host_case("Claustrophobia", CLAUSTROPHOBIA, CastPaymentMode::Auto);
}

/// DealDamage sibling: "it deals 2 damage to enchanted creature" never targets.
#[test]
fn fatal_attraction_etb_damage_does_not_retrigger_ward() {
    let mut sc = GameScenario::new();
    sc.at_phase(Phase::PreCombatMain);
    let host = sc
        .add_creature(P1, "Warded Brute", 4, 4)
        .with_keyword(Keyword::Ward(WardCost::Mana(ManaCost::generic(2))))
        .id();
    let aura = aura_in_hand(&mut sc, P0, "Fatal Attraction", FATAL_ATTRACTION);
    sc.add_creature(P0, "Grizzly Bears", 2, 2);
    sc.with_mana_pool(P0, pool(0, 2));
    let mut runner = sc.build();

    let d = cast_aura_and_resolve(&mut runner, aura, host, CastPaymentMode::Auto);
    let s = runner.state();
    assert_eq!(
        s.objects[&aura].attached_to,
        Some(AttachTarget::Object(host)),
        "reach: Fatal Attraction resolved and attached"
    );
    assert_eq!(d.ward_triggers(), 1, "ward triggers exactly once");
    assert_eq!(
        s.objects[&host].damage_marked, 2,
        "the ETB deals 2 damage to the enchanted creature"
    );
}

/// The "It" in "It deals damage to you equal to its power" names the host the
/// tap instruction acted on, so the chained damage follows the bound host.
#[test]
fn bind_the_monster_taps_host_and_host_deals_its_power_to_you() {
    let mut sc = GameScenario::new();
    sc.at_phase(Phase::PreCombatMain);
    let host = sc.add_creature(P1, "Craw Wurm", 6, 4).id();
    let aura = aura_in_hand(&mut sc, P0, "Bind the Monster", BIND_THE_MONSTER);
    sc.add_creature(P0, "Grizzly Bears", 2, 2);
    let mut runner = sc.build();
    let life_before = runner.state().players[P0.0 as usize].life;

    let d = cast_aura_and_resolve(&mut runner, aura, host, CastPaymentMode::Auto);
    let s = runner.state();
    assert!(
        d.targeted_aura_spell,
        "reach: the Aura spell chose its target"
    );
    assert_eq!(
        s.objects[&aura].attached_to,
        Some(AttachTarget::Object(host)),
        "reach: Bind the Monster resolved and attached"
    );
    assert!(
        s.objects[&host].tapped,
        "the ETB taps the enchanted creature"
    );
    assert_eq!(
        s.players[P0.0 as usize].life,
        life_before - 6,
        "the enchanted creature deals damage equal to its power to Bind the Monster's controller"
    );
}

/// Control-change sibling with an anaphor chain: "gain control of enchanted
/// permanent until end of turn. Untap that permanent. It gains haste until end of
/// turn." Ward triggers only for the Aura spell, and "that permanent"/"It" follow
/// the host the first instruction acted on.
#[test]
fn frenzied_fugue_steals_untaps_and_hastes_ward_host_with_one_ward_trigger() {
    let mut sc = GameScenario::new();
    sc.at_phase(Phase::PreCombatMain);
    let host = sc
        .add_creature(P1, "Warded Brute", 4, 4)
        .with_keyword(Keyword::Ward(WardCost::Mana(ManaCost::generic(2))))
        .id();
    let aura = aura_in_hand(&mut sc, P0, "Frenzied Fugue", FRENZIED_FUGUE);
    sc.add_creature(P0, "Grizzly Bears", 2, 2);
    sc.with_mana_pool(P0, pool(0, 2));
    let mut runner = sc.build();
    set_tapped(&mut runner, host, true);

    let d = cast_aura_and_resolve(&mut runner, aura, host, CastPaymentMode::Auto);
    let s = runner.state();
    assert_eq!(
        s.objects[&aura].attached_to,
        Some(AttachTarget::Object(host)),
        "reach: Frenzied Fugue resolved and attached"
    );
    assert_eq!(d.ward_triggers(), 1, "ward triggers exactly once");
    assert_eq!(
        s.objects[&host].controller, P0,
        "Fugue's controller gains control"
    );
    assert!(
        !s.objects[&host].tapped,
        "'Untap that permanent' untaps the host"
    );
    assert!(
        s.objects[&host]
            .keywords
            .iter()
            .any(|k| matches!(k, Keyword::Haste)),
        "'It gains haste' grants haste to the host"
    );
}

/// Conditional-sub sibling: "you may pay {3}. If you do, tap enchanted
/// permanent." The host binding happens when the "if you do" instruction runs.
#[test]
fn marooned_paid_taps_ward_host_with_one_ward_trigger() {
    let mut sc = GameScenario::new();
    sc.at_phase(Phase::PreCombatMain);
    let host = sc
        .add_creature(P1, "Patchwork Automaton", 1, 1)
        .with_keyword(Keyword::Ward(WardCost::Mana(ManaCost::generic(2))))
        .id();
    let aura = aura_in_hand(&mut sc, P0, "Marooned", MAROONED);
    sc.add_creature(P0, "Grizzly Bears", 2, 2);
    sc.with_mana_pool(P0, pool(0, 5));
    let mut runner = sc.build();

    let d = cast_aura_and_resolve(&mut runner, aura, host, CastPaymentMode::Auto);
    let s = runner.state();
    assert_eq!(
        s.objects[&aura].attached_to,
        Some(AttachTarget::Object(host)),
        "reach: Marooned resolved and attached"
    );
    assert_eq!(d.ward_triggers(), 1, "ward triggers exactly once");
    assert!(
        s.objects[&host].tapped,
        "the paid ETB taps the enchanted permanent"
    );
}

/// Optional + modal sibling: "{3}: You may tap or untap equipped creature." The
/// chosen tap/untap branch acts on the bound host (shrouded here).
#[test]
fn sword_of_the_paruns_untaps_a_shrouded_equipped_creature() {
    let mut sc = GameScenario::new();
    sc.at_phase(Phase::PreCombatMain);
    let host = sc
        .add_creature(P0, "Shrouded Bear", 2, 2)
        .with_keyword(Keyword::Shroud)
        .id();
    let sword = sc
        .add_creature(P0, "Sword of the Paruns", 0, 0)
        .as_artifact()
        .with_subtypes(vec!["Equipment"])
        .from_oracle_text(SWORD_OF_THE_PARUNS)
        .id();
    sc.with_mana_pool(P0, pool(0, 3));
    let mut runner = sc.build();
    attach(&mut runner, sword, host);
    set_tapped(&mut runner, host, true);

    let index = ability_index(&runner, sword, |e| matches!(e, Effect::TargetOnly { .. }));
    runner
        .act(GameAction::ActivateAbility {
            source_id: sword,
            ability_index: index,
        })
        .expect("CR 115.10a: 'tap or untap equipped creature' does not target");
    let mut chose_branch = false;
    for _ in 0..16 {
        let action = match &runner.state().waiting_for {
            WaitingFor::Priority { .. } if runner.state().stack.is_empty() => break,
            WaitingFor::Priority { .. } => GameAction::PassPriority,
            WaitingFor::OptionalEffectChoice { .. } => {
                GameAction::DecideOptionalEffect { accept: true }
            }
            WaitingFor::ChooseOneOfBranch { branches, .. } => {
                chose_branch = true;
                let untap = branches
                    .iter()
                    .position(|b| {
                        matches!(
                            &*b.effect,
                            Effect::SetTapState {
                                state: engine::types::ability::TapStateChange::Untap,
                                ..
                            }
                        )
                    })
                    .expect("an untap branch");
                GameAction::ChooseBranch { index: untap }
            }
            other => panic!("unexpected prompt: {other:?}"),
        };
        runner.act(action).expect("prompt accepts");
    }
    assert!(chose_branch, "reach: the tap-or-untap choice was offered");
    assert!(
        !runner.state().objects[&host].tapped,
        "the shrouded equipped creature is untapped"
    );
}

/// Puts the Aura or Equipment `text` on the battlefield attached to a shrouded
/// creature of P0's (tapped), with `mana` floating, and activates the ability
/// `pick` selects. CR 115.10a: the host is not a target, so shroud can't stop it.
fn activate_on_shrouded_host(
    text: &str,
    equipment: bool,
    mana: Vec<ManaUnit>,
    pick: fn(&Effect) -> bool,
) -> (GameRunner, ObjectId, ObjectId) {
    let mut sc = GameScenario::new();
    sc.at_phase(Phase::PreCombatMain);
    let host = sc
        .add_creature(P0, "Shrouded Bear", 2, 2)
        .with_keyword(Keyword::Shroud)
        .id();
    let source = if equipment {
        sc.add_creature(P0, "Test Equipment", 0, 0)
            .as_artifact()
            .with_subtypes(vec!["Equipment"])
            .from_oracle_text(text)
            .id()
    } else {
        aura_on_battlefield(&mut sc, P0, "Test Aura", text)
    };
    sc.with_mana_pool(P0, mana);
    let mut runner = sc.build();
    attach(&mut runner, source, host);
    let index = ability_index(&runner, source, pick);
    runner
        .act(GameAction::ActivateAbility {
            source_id: source,
            ability_index: index,
        })
        .expect("CR 115.10a: an ability naming the host without 'target' can be activated");
    runner.advance_until_stack_empty();
    assert!(
        runner.state().stack.is_empty(),
        "reach: the ability resolved"
    );
    (runner, source, host)
}

fn mana(kind: ManaType, n: usize) -> Vec<ManaUnit> {
    std::iter::repeat_n(ManaUnit::new(kind, ObjectId(0), false, vec![]), n).collect()
}

/// Bounce sibling: Sun Clasp, "{W}: Return enchanted creature to its owner's hand."
#[test]
fn sun_clasp_returns_its_shrouded_host() {
    let (runner, _, host) =
        activate_on_shrouded_host(SUN_CLASP, false, mana(ManaType::White, 1), |e| {
            matches!(e, Effect::Bounce { .. })
        });
    assert_eq!(runner.state().objects[&host].zone, Zone::Hand);
}

/// PhaseOut sibling: Vanishing, "{U}{U}: Enchanted creature phases out."
#[test]
fn vanishing_phases_out_its_shrouded_host() {
    let (runner, _, host) =
        activate_on_shrouded_host(VANISHING, false, mana(ManaType::Blue, 2), |e| {
            matches!(e, Effect::PhaseOut { .. })
        });
    assert!(
        !runner.state().objects[&host].is_phased_in(),
        "the host phased out"
    );
}

/// PutAtLibraryPosition sibling: Shattered Ego, "{3}{U}{U}: Put enchanted
/// creature into its owner's library third from the top."
#[test]
fn shattered_ego_puts_its_shrouded_host_into_the_library() {
    let mut pool = mana(ManaType::Blue, 2);
    pool.extend(mana(ManaType::Colorless, 3));
    let (runner, _, host) = activate_on_shrouded_host(SHATTERED_EGO, false, pool, |e| {
        matches!(e, Effect::PutAtLibraryPosition { .. })
    });
    assert_eq!(runner.state().objects[&host].zone, Zone::Library);
}

/// DoublePT sibling: Junk Jet, "{3}, Sacrifice another artifact: Double equipped
/// creature's power until end of turn."
#[test]
fn junk_jet_doubles_its_shrouded_equipped_creatures_power() {
    let mut sc = GameScenario::new();
    sc.at_phase(Phase::PreCombatMain);
    let host = sc
        .add_creature(P0, "Shrouded Bear", 3, 3)
        .with_keyword(Keyword::Shroud)
        .id();
    let jet = sc
        .add_creature(P0, "Junk Jet", 0, 0)
        .as_artifact()
        .with_subtypes(vec!["Equipment"])
        .from_oracle_text(JUNK_JET)
        .id();
    let fodder = sc.add_creature(P0, "Spare Part", 0, 0).as_artifact().id();
    sc.with_mana_pool(P0, mana(ManaType::Colorless, 3));
    let mut runner = sc.build();
    attach(&mut runner, jet, host);
    let index = ability_index(&runner, jet, |e| matches!(e, Effect::DoublePT { .. }));
    runner
        .act(GameAction::ActivateAbility {
            source_id: jet,
            ability_index: index,
        })
        .expect("CR 115.10a: 'double equipped creature's power' does not target");
    for _ in 0..8 {
        let action = match &runner.state().waiting_for {
            WaitingFor::Priority { .. } if runner.state().stack.is_empty() => break,
            WaitingFor::Priority { .. } => GameAction::PassPriority,
            WaitingFor::PayCost { choices, .. } => {
                assert_eq!(
                    choices,
                    &vec![fodder],
                    "reach: the other artifact is the fodder"
                );
                GameAction::SelectCards {
                    cards: vec![fodder],
                }
            }
            other => panic!("unexpected prompt: {other:?}"),
        };
        runner.act(action).expect("prompt accepts");
    }
    let s = runner.state();
    assert_eq!(
        s.objects[&fodder].zone,
        Zone::Graveyard,
        "reach: the artifact was sacrificed"
    );
    assert_eq!(
        s.objects[&host].power,
        Some(6),
        "the equipped creature's power doubled"
    );
}

/// CopyTokenOf sibling: Followed Footsteps, "At the beginning of your upkeep,
/// create a token that's a copy of enchanted creature." on a shrouded creature.
#[test]
fn followed_footsteps_copies_its_shrouded_host() {
    let mut sc = GameScenario::new();
    sc.at_phase(Phase::PreCombatMain);
    let host = sc
        .add_creature(P1, "Shrouded Bear", 2, 2)
        .with_keyword(Keyword::Shroud)
        .id();
    let aura = aura_on_battlefield(&mut sc, P0, "Followed Footsteps", FOLLOWED_FOOTSTEPS);
    sc.with_library_top(P0, &["Card A", "Card B"]);
    sc.with_library_top(P1, &["Card C", "Card D"]);
    let mut runner = sc.build();
    attach(&mut runner, aura, host);
    // From P1's end step to P0's upkeep.
    {
        let state = runner.state_mut();
        state.phase = Phase::End;
        state.active_player = P1;
        state.priority_player = P1;
        state.waiting_for = WaitingFor::Priority { player: P1 };
    }
    runner.advance_to_phase(Phase::Upkeep);
    runner.advance_until_stack_empty();
    let s = runner.state();
    assert_eq!(s.active_player, P0, "reach: it is P0's upkeep");
    let copies = s
        .battlefield
        .iter()
        .filter(|id| {
            let o = &s.objects[*id];
            o.is_token && o.name == "Shrouded Bear" && o.controller == P0
        })
        .count();
    assert_eq!(
        copies, 1,
        "P0 creates one token copy of the enchanted creature"
    );
}

/// Suspect sibling: Convenient Target, "When this Aura enters, suspect
/// enchanted creature." cast on an opponent's ward creature.
#[test]
fn convenient_target_suspects_ward_host_with_one_ward_trigger() {
    let mut sc = GameScenario::new();
    sc.at_phase(Phase::PreCombatMain);
    let host = sc
        .add_creature(P1, "Patchwork Automaton", 1, 1)
        .with_keyword(Keyword::Ward(WardCost::Mana(ManaCost::generic(2))))
        .id();
    let aura = aura_in_hand(&mut sc, P0, "Convenient Target", CONVENIENT_TARGET);
    sc.add_creature(P0, "Grizzly Bears", 2, 2);
    sc.with_mana_pool(P0, pool(0, 2));
    let mut runner = sc.build();

    let d = cast_aura_and_resolve(&mut runner, aura, host, CastPaymentMode::Auto);
    let s = runner.state();
    assert_eq!(
        s.objects[&aura].attached_to,
        Some(AttachTarget::Object(host)),
        "reach: Convenient Target resolved and attached"
    );
    assert_eq!(d.ward_triggers(), 1, "ward triggers exactly once");
    assert!(
        s.objects[&host].is_suspected,
        "the enchanted creature is suspected"
    );
}

/// Puts Claustrophobia onto the battlefield attached to `host` (as a resolved
/// Aura spell would) and leaves its ETB trigger waiting on the stack.
fn claustrophobia_etb_on_stack(
    extra_aura_host: bool,
) -> (GameRunner, ObjectId, ObjectId, Option<ObjectId>) {
    let mut sc = GameScenario::new();
    sc.at_phase(Phase::PreCombatMain);
    let host = sc.add_creature(P1, "Grizzly Bears", 2, 2).id();
    let bystander = extra_aura_host.then(|| sc.add_creature(P1, "Bystander", 2, 2).id());
    let aura = aura_in_hand(&mut sc, P0, "Claustrophobia", CLAUSTROPHOBIA);
    let other_aura = extra_aura_host
        .then(|| aura_on_battlefield(&mut sc, P1, "Rancor Dummy", "Enchant creature"));
    let mut runner = sc.build();
    if let (Some(other), Some(bystander)) = (other_aura, bystander) {
        attach(&mut runner, other, bystander);
    }
    runner.cast(aura).target_object(host).commit();
    // Resolve the Aura spell only; its ETB trigger goes on the stack.
    runner.act(GameAction::PassPriority).expect("pass");
    for _ in 0..8 {
        if runner.state().stack.iter().any(|e| {
            matches!(&e.kind, StackEntryKind::TriggeredAbility { source_id, .. } if *source_id == aura)
        }) {
            break;
        }
        runner.act(GameAction::PassPriority).expect("pass");
    }
    assert!(
        runner.state().stack.iter().any(|e| {
            matches!(&e.kind, StackEntryKind::TriggeredAbility { source_id, .. } if *source_id == aura)
        }),
        "reach: Claustrophobia's ETB trigger is on the stack"
    );
    assert_eq!(
        runner.state().objects[&aura].attached_to,
        Some(AttachTarget::Object(host)),
        "reach: Claustrophobia is attached to the host"
    );
    (runner, aura, host, bystander)
}

/// HOSTILE: the Aura leaves the battlefield while its ETB is on the stack. The
/// instruction uses the Aura's last known information (CR 608.2h), so the
/// creature it was attached to is still tapped.
#[test]
fn aura_bounced_in_response_still_taps_its_last_known_host() {
    let (mut runner, aura, host, _) = claustrophobia_etb_on_stack(false);
    let mut events = Vec::new();
    engine::game::zones::move_to_zone(runner.state_mut(), aura, Zone::Hand, &mut events);
    runner.advance_until_stack_empty();
    let s = runner.state();
    assert_eq!(
        s.objects[&aura].zone,
        Zone::Hand,
        "reach: the Aura left the battlefield"
    );
    assert!(
        s.objects[&host].tapped,
        "CR 608.2h: 'enchanted creature' is the creature the Aura was last attached to"
    );
}

/// HOSTILE: the host leaves the battlefield while the ETB is on the stack, and
/// another creature on that side has an unrelated Aura attached. The ETB taps
/// nothing; in particular it never re-resolves "enchanted creature" to some
/// other creature that happens to wear an Aura.
#[test]
fn host_gone_before_etb_resolves_taps_nothing() {
    let (mut runner, aura, host, bystander) = claustrophobia_etb_on_stack(true);
    let bystander = bystander.expect("bystander staged");
    let mut events = Vec::new();
    engine::game::zones::move_to_zone(runner.state_mut(), host, Zone::Hand, &mut events);
    runner.advance_until_stack_empty();
    let s = runner.state();
    assert_eq!(
        s.objects[&host].zone,
        Zone::Hand,
        "reach: the host left the battlefield"
    );
    assert!(
        !s.objects[&host].tapped,
        "the departed host is not tapped in its new zone"
    );
    assert!(
        !s.objects[&bystander].tapped,
        "an unrelated enchanted creature is not 'enchanted creature' for Claustrophobia"
    );
    assert_ne!(
        s.objects[&aura].zone,
        Zone::Battlefield,
        "reach: CR 303.4c put the now-unattached Aura into the graveyard"
    );
}

/// Activated sibling: "{U}: Untap enchanted creature." works on a host with
/// shroud — Pemmin's Aura's own third ability grants exactly that.
#[test]
fn pemmins_aura_untaps_a_shrouded_host() {
    let mut sc = GameScenario::new();
    sc.at_phase(Phase::PreCombatMain);
    let host = sc
        .add_creature(P0, "Shrouded Bear", 2, 2)
        .with_keyword(Keyword::Shroud)
        .id();
    let aura = aura_on_battlefield(&mut sc, P0, "Pemmin's Aura", PEMMINS_AURA);
    sc.with_mana_pool(P0, pool(1, 0));
    let mut runner = sc.build();
    attach(&mut runner, aura, host);
    set_tapped(&mut runner, host, true);

    let index = ability_index(&runner, aura, |e| matches!(e, Effect::SetTapState { .. }));
    runner
        .act(GameAction::ActivateAbility {
            source_id: aura,
            ability_index: index,
        })
        .expect("CR 115.10a: 'untap enchanted creature' does not target, so shroud cannot stop it");
    runner.advance_until_stack_empty();
    assert!(
        !runner.state().objects[&host].tapped,
        "the shrouded enchanted creature is untapped"
    );
}

fn cloak_board(attached: bool) -> (GameRunner, ObjectId, ObjectId, ObjectId) {
    let mut sc = GameScenario::new();
    sc.at_phase(Phase::PreCombatMain);
    let host = sc
        .add_creature(P0, "Shrouded Bear", 2, 2)
        .with_keyword(Keyword::Shroud)
        .id();
    let other = sc.add_creature(P0, "Equipped Elsewhere", 2, 2).id();
    let cloak = sc
        .add_creature(P0, "Cloak of the Philosopher", 0, 0)
        .as_artifact()
        .with_subtypes(vec!["Equipment"])
        .from_oracle_text(CLOAK_OF_THE_PHILOSOPHER)
        .id();
    let other_equipment = sc
        .add_creature(P0, "Other Blade", 0, 0)
        .as_artifact()
        .with_subtypes(vec!["Equipment"])
        .from_oracle_text("Equipped creature gets +1/+1.\nEquip {1}")
        .id();
    sc.with_mana_pool(P0, pool(0, 2));
    let mut runner = sc.build();
    if attached {
        attach(&mut runner, cloak, host);
    }
    attach(&mut runner, other_equipment, other);
    set_tapped(&mut runner, host, true);
    set_tapped(&mut runner, other, true);
    (runner, cloak, host, other)
}

/// Equipment sibling: "{2}: Untap equipped creature." on a shrouded host.
#[test]
fn cloak_of_the_philosopher_untaps_a_shrouded_equipped_creature() {
    let (mut runner, cloak, host, other) = cloak_board(true);
    let index = ability_index(&runner, cloak, |e| matches!(e, Effect::SetTapState { .. }));
    runner
        .act(GameAction::ActivateAbility {
            source_id: cloak,
            ability_index: index,
        })
        .expect("CR 115.10a: 'untap equipped creature' does not target");
    runner.advance_until_stack_empty();
    let s = runner.state();
    assert!(
        !s.objects[&host].tapped,
        "the equipped creature is untapped"
    );
    assert!(
        s.objects[&other].tapped,
        "a creature equipped by a different Equipment is untouched"
    );
}

/// CR 301.5c + CR 301.5f: an unattached Equipment has no equipped creature, so
/// the ability resolves and untaps nothing (it never falls back to "some
/// creature with an Equipment attached").
#[test]
fn unattached_cloak_untaps_nothing() {
    let (mut runner, cloak, host, other) = cloak_board(false);
    let index = ability_index(&runner, cloak, |e| matches!(e, Effect::SetTapState { .. }));
    runner
        .act(GameAction::ActivateAbility {
            source_id: cloak,
            ability_index: index,
        })
        .expect("the ability names no target, so it can be activated");
    runner.advance_until_stack_empty();
    let s = runner.state();
    assert!(s.stack.is_empty(), "reach: the ability resolved");
    assert!(
        s.objects[&host].tapped,
        "no creature is equipped by the Cloak"
    );
    assert!(
        s.objects[&other].tapped,
        "another Equipment's creature is untouched"
    );
}

/// LKI sibling (Destroy): "When this Aura leaves the battlefield, destroy
/// enchanted creature." — the Aura is already gone when the trigger resolves.
#[test]
fn parallax_dementia_leaves_trigger_destroys_last_known_host() {
    let mut sc = GameScenario::new();
    sc.at_phase(Phase::PreCombatMain);
    let host = sc
        .add_creature(P1, "Shrouded Bear", 2, 2)
        .with_keyword(Keyword::Shroud)
        .id();
    let aura = aura_on_battlefield(&mut sc, P0, "Parallax Dementia", PARALLAX_DEMENTIA_LEAVES);
    let mut runner = sc.build();
    attach(&mut runner, aura, host);
    let mut events = Vec::new();
    engine::game::zones::move_to_zone(runner.state_mut(), aura, Zone::Graveyard, &mut events);
    engine::game::triggers::process_triggers(runner.state_mut(), &events);
    assert!(
        runner.state().stack.iter().any(|e| {
            matches!(&e.kind, StackEntryKind::TriggeredAbility { source_id, .. } if *source_id == aura)
        }),
        "reach: the leaves-the-battlefield trigger went on the stack"
    );
    runner.advance_until_stack_empty();
    assert_eq!(
        runner.state().objects[&host].zone,
        Zone::Graveyard,
        "CR 608.2h + CR 115.10a: the shrouded creature the Aura was attached to is destroyed"
    );
}

fn has_regeneration_shield(runner: &GameRunner, id: ObjectId) -> bool {
    runner.state().objects[&id]
        .replacement_definitions
        .iter_unchecked()
        .any(|r| r.description.as_deref() == Some("Regenerate"))
}

/// Nurturing Licid's "{G}: Regenerate enchanted creature." in each of its forms.
/// As an Aura it shields the creature it is attached to. As a creature it is
/// attached to nothing, so there is no enchanted creature: the ability must not
/// fall back to "every creature that has some Aura attached".
fn nurturing_licid_case(attached_as_aura: bool) -> (bool, bool) {
    let mut sc = GameScenario::new();
    sc.at_phase(Phase::PreCombatMain);
    let host = sc.add_creature(P0, "Licid Host", 2, 2).id();
    let bystander = sc.add_creature(P1, "Enchanted Bystander", 2, 2).id();
    let other_aura = aura_on_battlefield(&mut sc, P1, "Unrelated Aura", "Enchant creature");
    let licid = sc
        .add_creature_from_oracle(P0, "Nurturing Licid", 1, 1, NURTURING_LICID)
        .id();
    sc.with_mana_pool(
        P0,
        vec![ManaUnit::new(ManaType::Green, ObjectId(0), false, vec![])],
    );
    let mut runner = sc.build();
    attach(&mut runner, other_aura, bystander);
    if attached_as_aura {
        // The state its first ability leaves it in: an Aura enchantment, not a
        // creature (written to the base types so the layer pass keeps it).
        let obj = runner.state_mut().objects.get_mut(&licid).unwrap();
        obj.card_types
            .core_types
            .retain(|t| *t != engine::types::card_type::CoreType::Creature);
        obj.card_types
            .core_types
            .push(engine::types::card_type::CoreType::Enchantment);
        obj.card_types.subtypes = vec!["Aura".to_string()];
        obj.base_card_types = obj.card_types.clone();
        attach(&mut runner, licid, host);
    }
    let index = ability_index(&runner, licid, |e| matches!(e, Effect::Regenerate { .. }));
    runner
        .act(GameAction::ActivateAbility {
            source_id: licid,
            ability_index: index,
        })
        .expect("'regenerate enchanted creature' names no target, so it can be activated");
    runner.advance_until_stack_empty();
    assert!(
        runner.state().stack.is_empty(),
        "reach: the ability resolved"
    );
    (
        has_regeneration_shield(&runner, host),
        has_regeneration_shield(&runner, bystander),
    )
}

#[test]
fn nurturing_licid_as_aura_regenerates_its_host() {
    let (host, bystander) = nurturing_licid_case(true);
    assert!(
        host,
        "the creature the Licid enchants gets the regeneration shield"
    );
    assert!(
        !bystander,
        "a creature wearing a different Aura gets nothing"
    );
}

#[test]
fn nurturing_licid_as_creature_regenerates_nothing() {
    let (host, bystander) = nurturing_licid_case(false);
    assert!(!host, "reach: the Licid enchants nothing");
    assert!(
        !bystander,
        "CR 303.4b: an unattached Licid has no enchanted creature, so a creature wearing an \
         unrelated Aura is not regenerated"
    );
}

/// SYNTHETIC (fail-closed guard): a non-Aura source that is attached to nothing
/// names no enchanted creature, so "Enchanted creature phases out" affects
/// nothing. PhaseOut's resolver falls back to a filter scan when it has no
/// targets; without the no-host skip it would phase out an unrelated creature
/// that happens to wear an Aura.
#[test]
fn hostless_phase_out_affects_no_unrelated_enchanted_creature() {
    let mut sc = GameScenario::new();
    sc.at_phase(Phase::PreCombatMain);
    let source = sc
        .add_creature_from_oracle(
            P0,
            "Unattached Phaser",
            1,
            1,
            "{U}{U}: Enchanted creature phases out.",
        )
        .id();
    let bystander = sc.add_creature(P1, "Enchanted Bystander", 2, 2).id();
    let other_aura = aura_on_battlefield(&mut sc, P1, "Unrelated Aura", "Enchant creature");
    sc.with_mana_pool(P0, mana(ManaType::Blue, 2));
    let mut runner = sc.build();
    attach(&mut runner, other_aura, bystander);
    let index = ability_index(&runner, source, |e| matches!(e, Effect::PhaseOut { .. }));
    assert_eq!(
        runner.state().objects[&source].abilities[index].target_choice_timing,
        TargetChoiceTiming::Resolution,
        "reach: the host reference is resolution-timed"
    );
    runner
        .act(GameAction::ActivateAbility {
            source_id: source,
            ability_index: index,
        })
        .expect("the ability names no target, so it can be activated");
    runner.advance_until_stack_empty();
    let s = runner.state();
    assert!(s.stack.is_empty(), "reach: the ability resolved");
    assert!(
        s.objects[&bystander].is_phased_in(),
        "a creature wearing an unrelated Aura is not the source's enchanted creature"
    );
}

/// SHAPE (building block): the literal word "target" is the only thing that
/// makes an Aura's own instruction stack-targeted (CR 115.10a).
#[test]
fn host_reference_is_resolution_timed_and_literal_target_stays_stack() {
    let mut sc = GameScenario::new();
    let described = aura_on_battlefield(
        &mut sc,
        P0,
        "Described",
        "Enchant creature\n{1}: Tap enchanted creature.",
    );
    let targeted = aura_on_battlefield(
        &mut sc,
        P0,
        "Targeted",
        "Enchant creature\n{1}: Tap target creature.",
    );
    let runner = sc.build();
    let timing = |id: ObjectId| runner.state().objects[&id].abilities[0].target_choice_timing;
    assert_eq!(timing(described), TargetChoiceTiming::Resolution);
    assert_eq!(timing(targeted), TargetChoiceTiming::Stack);
}
