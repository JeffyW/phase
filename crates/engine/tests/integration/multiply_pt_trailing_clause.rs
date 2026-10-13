//! A same-sentence tail after a P/T multiply ("double its power and toughness and
//! this Saga gains hexproof") must never be dropped while the clause is reported
//! as supported (issue #9777). CR 608.2c: the controller follows all of an
//! ability's instructions in the order written. CR 701.10a: doubling P/T is a
//! continuous effect.
//!
//! No printed card carries an unparsed tail after a P/T multiply. World War Hulk
//! III ("Until end of turn, double its power and toughness and it gains trample.")
//! is the closest printed card and the green control: its "it gains" conjunct is
//! split off before the multiply parser sees the clause. The synthetic chapters
//! swap that conjunct for a grant to the Saga itself.

use engine::parser::oracle::parse_oracle_text;
use engine::types::ability::{AbilityDefinition, Effect};

const SAGA_REMINDER: &str =
    "(As this Saga enters and after your draw step, add a lore counter. Sacrifice after III.)";
const HULK_I_II: &str = "I \u{2014} The next red or green creature spell you cast this turn can be cast without paying its mana cost.\nII \u{2014} Put three +1/+1 counters on target creature you control.";

/// Every effect node in an ability chain: sub, else, and mode branches.
fn collect(def: &AbilityDefinition, out: &mut Vec<Effect>) {
    out.push((*def.effect).clone());
    if let Some(sub) = def.sub_ability.as_deref() {
        collect(sub, out);
    }
    if let Some(other) = def.else_ability.as_deref() {
        collect(other, out);
    }
    for mode in &def.mode_abilities {
        collect(mode, out);
    }
}

fn hulk_chapter_three(name: &str, chapter_three: &str) -> Vec<Effect> {
    let oracle = format!("{SAGA_REMINDER}\n{HULK_I_II}\nIII \u{2014} {chapter_three}");
    let parsed = parse_oracle_text(
        &oracle,
        name,
        &[],
        &["Enchantment".to_string()],
        &["Saga".to_string()],
    );
    let trigger = parsed
        .triggers
        .iter()
        .find(|t| t.saga_chapter == Some(3))
        .unwrap_or_else(|| panic!("reach: {name} has a chapter III trigger: {parsed:#?}"));
    let mut out = Vec::new();
    collect(
        trigger
            .execute
            .as_deref()
            .expect("chapter III has an ability"),
        &mut out,
    );
    out
}

/// Every effect node a creature card's abilities and triggers produce.
fn creature_effects(name: &str, oracle: &str) -> Vec<Effect> {
    let parsed = parse_oracle_text(
        oracle,
        name,
        &[],
        &["Creature".to_string()],
        &["Ape".to_string()],
    );
    let mut out = Vec::new();
    for ability in &parsed.abilities {
        collect(ability, &mut out);
    }
    for trigger in &parsed.triggers {
        if let Some(execute) = trigger.execute.as_deref() {
            collect(execute, &mut out);
        }
    }
    out
}

fn is_multiply_pt(effect: &Effect) -> bool {
    matches!(effect, Effect::DoublePT { .. } | Effect::DoublePTAll { .. })
}

/// The explicit gap the multiply-P/T parser emits for an unparsed tail, carrying
/// the clause it refused.
fn is_trailing_gap(effect: &Effect, fragment: &str) -> bool {
    matches!(effect, Effect::Unimplemented { name, description: Some(d) }
        if name == "multiply_pt_trailing_clause" && d.contains(fragment))
}

/// Control. Printed World War Hulk III keeps its DoublePT and its trample grant.
#[test]
fn world_war_hulk_keeps_double_pt_and_its_gains_conjunct() {
    let nodes = hulk_chapter_three(
        "World War Hulk",
        "Choose target creature you control. Until end of turn, double its power and toughness and it gains trample.",
    );
    assert!(
        nodes.iter().any(is_multiply_pt),
        "chapter III doubles P/T, got {nodes:#?}"
    );
    assert!(
        nodes
            .iter()
            .any(|e| matches!(e, Effect::GenericEffect { .. })),
        "\"and it gains trample\" lowers to a grant, got {nodes:#?}"
    );
    assert!(
        nodes
            .iter()
            .all(|e| e.unimplemented_description().is_none()),
        "the printed chapter has no gap, got {nodes:#?}"
    );
}

/// Defect. A grant to the Saga in the same sentence as the doubling fails the
/// clause closed instead of vanishing behind a DoublePT.
#[test]
fn saga_self_grant_after_double_pt_fails_closed() {
    for tail in [
        "this Saga gains hexproof.",
        "this Saga gains \"{T}: Add {C}.\"",
    ] {
        let nodes = hulk_chapter_three(
            "Synthetic Hulk Saga",
            &format!(
                "Choose target creature you control. Until end of turn, double its power and toughness and {tail}"
            ),
        );
        assert!(
            matches!(nodes.first(), Some(Effect::TargetOnly { .. })),
            "reach: chapter III with tail {tail:?} parses its leading choice, got {nodes:#?}"
        );
        assert!(
            nodes
                .iter()
                .any(|e| is_trailing_gap(e, "double its power and toughness and ~ gains")),
            "the tail {tail:?} must fail the doubling clause closed, got {nodes:#?}"
        );
        assert!(
            !nodes.iter().any(is_multiply_pt),
            "a DoublePT must not claim a clause whose tail it dropped, got {nodes:#?}"
        );
    }
}

/// Class. Every multiply-P/T arm and both multiplier words fail closed on an
/// unparsed same-sentence tail outside a Saga, while the printed form of each arm
/// (tail-free) still parses with no gap.
#[test]
fn every_multiply_pt_arm_fails_closed_on_a_trailing_tail() {
    let arms = [
        // possessive target (Bulk Up)
        "{1}: Double target creature's power",
        // self possessive (Targ Nar)
        "{1}: Double Synthetic Ape's power and toughness",
        // the … of target (Unleash Fury)
        "{1}: Double the power of target creature",
        // the … of each (Roar of Endless Song III)
        "{1}: Double the power and toughness of each creature you control",
        // anaphoric its (Grunn)
        "Whenever Synthetic Ape attacks, double its power and toughness",
        // triple (Tifa's Limit Break, Final Heaven)
        "{1}: Triple target creature's power and toughness",
    ];
    for head in arms {
        let printed = format!("{head} until end of turn.");
        let nodes = creature_effects("Synthetic Ape", &printed);
        assert!(
            nodes.iter().any(is_multiply_pt),
            "reach: {printed:?} parses as a P/T multiply, got {nodes:#?}"
        );
        assert!(
            nodes
                .iter()
                .all(|e| e.unimplemented_description().is_none()),
            "{printed:?} has no gap, got {nodes:#?}"
        );

        let tailed = format!("{head} and Synthetic Ape gains hexproof until end of turn.");
        let nodes = creature_effects("Synthetic Ape", &tailed);
        assert!(
            nodes
                .iter()
                .any(|e| is_trailing_gap(e, "and ~ gains hexproof")),
            "{tailed:?} must fail closed, got {nodes:#?}"
        );
        assert!(
            !nodes.iter().any(is_multiply_pt),
            "{tailed:?} must not report a P/T multiply, got {nodes:#?}"
        );
    }
}
