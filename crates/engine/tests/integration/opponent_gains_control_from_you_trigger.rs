//! CR 603.2 + CR 603.10d: "Whenever an opponent gains control of a permanent
//! from you" (Zidane, Tantalus Thief) — the opponent-gains half of the
//! look-back control-change triggers, driven through real control-changing
//! spells and the cleanup reversion.

use engine::game::scenario::{GameRunner, GameScenario, P0, P1};
use engine::types::format::FormatConfig;
use engine::types::game_state::WaitingFor;
use engine::types::identifiers::ObjectId;
use engine::types::mana::{ManaCost, ManaCostShard, ManaType, ManaUnit};
use engine::types::phase::Phase;
use engine::types::player::PlayerId;
use engine::types::zones::Zone;

const P2: PlayerId = PlayerId(2);

/// Verbatim Oracle text (Scryfall).
const ZIDANE: &str = "When Zidane enters, gain control of target creature an opponent \
    controls until end of turn. Untap it. It gains lifelink and haste until end of turn.\n\
    Whenever an opponent gains control of a permanent from you, you create a Treasure token.";
const ACT_OF_TREASON: &str = "Gain control of target creature until end of turn. Untap that \
    creature. It gains haste until end of turn. (It can attack and {T} this turn.)";
const MOB_RULE: &str = "Choose one \u{2014}\n\u{2022} Gain control of all creatures with power \
    4 or greater until end of turn. Untap those creatures. They gain haste until end of turn.\n\
    \u{2022} Gain control of all creatures with power 3 or less until end of turn. Untap those \
    creatures. They gain haste until end of turn.";
const SWITCHEROO: &str = "Exchange control of two target creatures.";
const PERPLEXING_CHIMERA: &str = "Whenever an opponent casts a spell, you may exchange control \
    of this creature and that spell. If you do, you may choose new targets for the spell. (If \
    the spell becomes a permanent, you control that permanent.)";

fn treasures(runner: &GameRunner, player: PlayerId) -> usize {
    let state = runner.state();
    state
        .battlefield
        .iter()
        .filter(|id| {
            let obj = &state.objects[*id];
            obj.name == "Treasure" && obj.controller == player
        })
        .count()
}

fn controller(runner: &GameRunner, id: ObjectId) -> PlayerId {
    runner.state().objects[&id].controller
}

fn give_priority(runner: &mut GameRunner, player: PlayerId) {
    let state = runner.state_mut();
    state.active_player = player;
    state.priority_player = player;
    state.waiting_for = WaitingFor::Priority { player };
}

/// Zidane with its printed mana cost, {3}{R}{W} (mana value 5).
fn zidane_cost() -> ManaCost {
    ManaCost::Cost {
        shards: vec![ManaCostShard::Red, ManaCostShard::White],
        generic: 3,
    }
}

fn add_zidane(scenario: &mut GameScenario, player: PlayerId) -> ObjectId {
    scenario
        .add_creature_from_oracle(player, "Zidane, Tantalus Thief", 3, 3, ZIDANE)
        .with_mana_cost(zidane_cost())
        .id()
}

fn add_free_spell(
    scenario: &mut GameScenario,
    player: PlayerId,
    name: &str,
    text: &str,
) -> ObjectId {
    scenario
        .add_spell_to_hand_from_oracle(player, name, false, text)
        .with_mana_cost(ManaCost::zero())
        .id()
}

/// An opponent stealing your permanent fires once for you; the same permanent
/// reverting back to you at cleanup is a gain TO you and must not fire again.
#[test]
fn opponent_steal_fires_and_revert_back_to_you_does_not() {
    let mut scenario = GameScenario::new();
    scenario.at_phase(Phase::PostCombatMain);
    add_zidane(&mut scenario, P0);
    let victim = scenario.add_vanilla(P0, 2, 2);
    let steal = add_free_spell(&mut scenario, P1, "Act of Treason", ACT_OF_TREASON);
    let mut runner = scenario.build();
    give_priority(&mut runner, P1);

    runner.cast(steal).target_object(victim).resolve();
    runner.advance_until_stack_empty();
    assert_eq!(
        controller(&runner, victim),
        P1,
        "REACH: P1 stole the creature"
    );
    assert_eq!(
        treasures(&runner, P0),
        1,
        "the steal creates one Treasure for P0"
    );
    assert_eq!(treasures(&runner, P1), 0);

    runner.advance_to_phase(Phase::Upkeep);
    runner.advance_until_stack_empty();
    assert_eq!(
        controller(&runner, victim),
        P0,
        "REACH: control reverted to P0 at cleanup"
    );
    assert_eq!(
        treasures(&runner, P0),
        1,
        "regaining control is not an opponent gaining control from you"
    );
}

/// Zidane's own steal is a gain TO you (no Treasure); when it ends at cleanup
/// the opponent regains control from you, which does fire (CR 514.3a: the
/// trigger is handled in an extra cleanup step).
#[test]
fn zidane_steal_returning_at_end_of_turn_fires() {
    let mut scenario = GameScenario::new();
    scenario.at_phase(Phase::PostCombatMain);
    let zidane = scenario
        .add_creature_to_hand_from_oracle(P0, "Zidane, Tantalus Thief", 3, 3, ZIDANE)
        .with_mana_cost(ManaCost::zero())
        .id();
    let bear = scenario.add_vanilla(P1, 2, 2);
    let mut runner = scenario.build();

    runner.cast(zidane).target_object(bear).resolve();
    runner.advance_until_stack_empty();
    assert_eq!(
        controller(&runner, bear),
        P0,
        "REACH: Zidane's ETB stole the bear"
    );
    assert_eq!(
        treasures(&runner, P0),
        0,
        "gaining control yourself does not fire"
    );

    runner.advance_to_phase(Phase::Upkeep);
    runner.advance_until_stack_empty();
    assert_eq!(
        controller(&runner, bear),
        P1,
        "REACH: the bear went back to P1"
    );
    assert_eq!(
        treasures(&runner, P0),
        1,
        "the opponent regaining control from you creates a Treasure"
    );
    assert_eq!(treasures(&runner, P1), 0);
}

/// CR 603.10d + CR 603.3a: the trigger looks back in time, so stealing Zidane
/// itself fires, controlled by its previous controller (Scryfall ruling).
#[test]
fn stealing_zidane_itself_fires_for_its_previous_controller() {
    let mut scenario = GameScenario::new();
    scenario.at_phase(Phase::PreCombatMain);
    let zidane = add_zidane(&mut scenario, P0);
    let steal = add_free_spell(&mut scenario, P1, "Act of Treason", ACT_OF_TREASON);
    let mut runner = scenario.build();
    give_priority(&mut runner, P1);

    runner.cast(steal).target_object(zidane).resolve();
    runner.advance_until_stack_empty();
    assert_eq!(controller(&runner, zidane), P1, "REACH: P1 stole Zidane");
    assert_eq!(
        treasures(&runner, P0),
        1,
        "P0 (the player who lost Zidane) gets the Treasure"
    );
    assert_eq!(treasures(&runner, P1), 0, "the new controller gets nothing");
}

/// CR 603.2c: a mass steal is one occurrence per permanent. Mob Rule's first
/// mode takes every creature with power 4 or greater: both 4/4s, not Zidane
/// (3/3).
#[test]
fn mass_steal_fires_once_per_permanent() {
    let mut scenario = GameScenario::new();
    scenario.at_phase(Phase::PreCombatMain);
    let zidane = add_zidane(&mut scenario, P0);
    let giants = [
        scenario.add_vanilla(P0, 4, 4),
        scenario.add_vanilla(P0, 4, 4),
    ];
    let mob_rule = add_free_spell(&mut scenario, P1, "Mob Rule", MOB_RULE);
    let mut runner = scenario.build();
    give_priority(&mut runner, P1);

    runner.cast(mob_rule).modes(&[0]).resolve();
    runner.advance_until_stack_empty();
    assert_eq!(
        controller(&runner, zidane),
        P0,
        "REACH: Zidane stayed with P0"
    );
    for giant in giants {
        assert_eq!(controller(&runner, giant), P1, "REACH: each 4/4 stolen");
    }
    assert_eq!(
        treasures(&runner, P0),
        2,
        "one Treasure per stolen permanent"
    );
    assert_eq!(treasures(&runner, P1), 0);
}

/// FAIL-CLOSED LIMIT, documented: a simultaneous mass steal that also takes
/// Zidane. The ruling ("If an opponent gains control of Zidane and one or more
/// other permanents from you simultaneously, Zidane's last ability will
/// trigger for each of those permanents, including itself.") gives P0 two
/// Treasures here. Collection can't tell when Zidane changed hands relative
/// to the bear, so the bear's trigger doesn't fire. This pins only that
/// nothing fires for the wrong player: Zidane's own trigger fires for P0, and
/// P1 (Zidane's new controller) gets nothing. Mob Rule's second mode takes
/// every creature with power 3 or less: Zidane (3/3, printed mana value 5) and
/// the bear.
#[test]
fn limit_mass_steal_taking_zidane_never_fires_for_the_wrong_player() {
    let mut scenario = GameScenario::new();
    scenario.at_phase(Phase::PreCombatMain);
    let zidane = add_zidane(&mut scenario, P0);
    let bear = scenario.add_vanilla(P0, 2, 2);
    let mob_rule = add_free_spell(&mut scenario, P1, "Mob Rule", MOB_RULE);
    let mut runner = scenario.build();
    give_priority(&mut runner, P1);
    assert_eq!(
        runner.state().objects[&zidane].mana_cost.mana_value(),
        5,
        "REACH: Zidane keeps its printed mana value"
    );

    runner.cast(mob_rule).modes(&[1]).resolve();
    runner.advance_until_stack_empty();
    assert_eq!(controller(&runner, zidane), P1, "REACH: Zidane stolen");
    assert_eq!(controller(&runner, bear), P1, "REACH: bear stolen");
    assert!(
        treasures(&runner, P0) >= 1,
        "Zidane's own trigger fires for the player who lost it"
    );
    assert_eq!(
        treasures(&runner, P1),
        0,
        "nothing fires for Zidane's new controller"
    );
}

/// CR 110.1 + CR 109.4: a spell changing control on the stack is not a
/// permanent. Zidane is on P1's side; P0's Perplexing Chimera exchanges itself
/// (P0 → P1, not from P1) for P1's creature spell (P1 → P0, a spell).
#[test]
fn spell_control_exchange_is_not_a_permanent() {
    let mut scenario = GameScenario::new();
    scenario.at_phase(Phase::PreCombatMain);
    add_zidane(&mut scenario, P1);
    let chimera = scenario
        .add_creature_from_oracle(P0, "Perplexing Chimera", 3, 3, PERPLEXING_CHIMERA)
        .id();
    let bears = scenario
        .add_creature_to_hand_from_oracle(P1, "Grizzly Bears", 2, 2, "")
        .with_mana_cost(ManaCost::zero())
        .id();
    let mut runner = scenario.build();
    give_priority(&mut runner, P1);

    runner.cast(bears).accept_optional().resolve();
    runner.advance_until_stack_empty();
    assert_eq!(
        controller(&runner, chimera),
        P1,
        "REACH: the Chimera swapped to P1"
    );
    assert!(
        runner.state().battlefield.contains(&bears),
        "REACH: the exchanged spell resolved"
    );
    assert_eq!(
        controller(&runner, bears),
        P0,
        "REACH: the spell resolved under P0"
    );
    assert_eq!(
        treasures(&runner, P1),
        0,
        "P0 gaining control of P1's spell is not gaining control of a permanent"
    );
}

/// CR 603.10d + CR 701.12a: exchanging Zidane for an opponent's creature is
/// one simultaneous control change. Zidane leaving you fires for you (its
/// controller before the exchange); the opponent's creature arriving does not,
/// and must not be credited to the opponent who now controls Zidane.
#[test]
fn exchanging_zidane_fires_only_for_its_previous_controller() {
    let mut scenario = GameScenario::new();
    scenario.at_phase(Phase::PreCombatMain);
    let zidane = add_zidane(&mut scenario, P0);
    let bear = scenario.add_vanilla(P1, 2, 2);
    let switcheroo = add_free_spell(&mut scenario, P0, "Switcheroo", SWITCHEROO);
    let mut runner = scenario.build();

    runner
        .cast(switcheroo)
        .target_objects(&[zidane, bear])
        .resolve();
    runner.advance_until_stack_empty();
    assert_eq!(controller(&runner, zidane), P1, "REACH: Zidane went to P1");
    assert_eq!(controller(&runner, bear), P0, "REACH: the bear came to P0");
    assert_eq!(treasures(&runner, P0), 1, "P0 lost Zidane to an opponent");
    assert_eq!(
        treasures(&runner, P1),
        0,
        "P1 did not control Zidane when P1 lost the bear"
    );
}

/// CR 102.3: in Two-Headed Giant a teammate is not an opponent, so a teammate
/// taking your permanent does not fire; an opponent taking it does.
#[test]
fn teammate_steal_does_not_fire() {
    for (thief, expected) in [(P1, 0), (P2, 1)] {
        let mut scenario = GameScenario::new_with_format(FormatConfig::two_headed_giant(), 4, 42);
        scenario.at_phase(Phase::PreCombatMain);
        add_zidane(&mut scenario, P0);
        let victim = scenario.add_vanilla(P0, 2, 2);
        let steal = add_free_spell(&mut scenario, thief, "Act of Treason", ACT_OF_TREASON);
        let mut runner = scenario.build();
        give_priority(&mut runner, thief);

        runner.cast(steal).target_object(victim).resolve();
        runner.advance_until_stack_empty();
        assert_eq!(
            controller(&runner, victim),
            thief,
            "REACH: {thief:?} took the creature"
        );
        assert_eq!(
            treasures(&runner, P0),
            expected,
            "thief {thief:?}: only an opponent gaining control fires"
        );
    }
}

/// A steal between two other players is not "from you"; the same steal of
/// your creature in the same three-player game is.
#[test]
fn steal_between_other_players_does_not_fire() {
    for (victim_controller, expected) in [(P2, 0), (P0, 1)] {
        let mut scenario = GameScenario::new_n_player(3, 42);
        scenario.at_phase(Phase::PreCombatMain);
        add_zidane(&mut scenario, P0);
        let victim = scenario.add_vanilla(victim_controller, 2, 2);
        let steal = add_free_spell(&mut scenario, P1, "Act of Treason", ACT_OF_TREASON);
        let mut runner = scenario.build();
        give_priority(&mut runner, P1);

        runner.cast(steal).target_object(victim).resolve();
        runner.advance_until_stack_empty();
        assert_eq!(
            controller(&runner, victim),
            P1,
            "REACH: P1 stole {victim_controller:?}'s creature"
        );
        assert_eq!(
            treasures(&runner, P0),
            expected,
            "victim controller {victim_controller:?}: only a steal from you fires"
        );
        assert_eq!(treasures(&runner, P2), 0);
    }
}

/// Synthetic "~" subject through the same parser arm: `valid_card` scopes the
/// changing permanent to the source itself.
const SELF_WATCHER: &str =
    "Whenever an opponent gains control of ~ from you, you create a Treasure token.";

#[test]
fn self_subject_fires_only_for_the_source() {
    for (steal_source, expected) in [(false, 0), (true, 1)] {
        let mut scenario = GameScenario::new();
        scenario.at_phase(Phase::PreCombatMain);
        let watcher = scenario
            .add_creature_from_oracle(P0, "Self Watcher", 1, 1, SELF_WATCHER)
            .id();
        let other = scenario.add_vanilla(P0, 2, 2);
        let victim = if steal_source { watcher } else { other };
        let steal = add_free_spell(&mut scenario, P1, "Act of Treason", ACT_OF_TREASON);
        let mut runner = scenario.build();
        give_priority(&mut runner, P1);

        runner.cast(steal).target_object(victim).resolve();
        runner.advance_until_stack_empty();
        assert_eq!(controller(&runner, victim), P1, "REACH: P1 stole it");
        assert_eq!(
            treasures(&runner, P0),
            expected,
            "steal_source {steal_source}: only the source changing hands fires"
        );
        assert_eq!(treasures(&runner, P1), 0);
    }
}

/// Verbatim Oracle text (Scryfall).
const HOMEWARD_PATH: &str =
    "{T}: Add {C}.\n{T}: Each player gains control of all creatures they own.";

/// CR 101.4 + CR 608.2e: Homeward Path returns every creature to its owner at
/// once. After Switcheroo gave P1 Zidane and P0 P1's bear, Homeward Path moves
/// Zidane back from P1 (an opponent of P1 gains control of it from P1:
/// Treasure for P1) and the bear back from P0 (P0 didn't control Zidane then:
/// nothing for P0).
#[test]
fn homeward_path_returns_fire_for_zidanes_controller_before_the_return() {
    let mut scenario = GameScenario::new();
    scenario.at_phase(Phase::PreCombatMain);
    let zidane = add_zidane(&mut scenario, P0);
    let bear = scenario.add_vanilla(P1, 2, 2);
    let switcheroo = add_free_spell(&mut scenario, P0, "Switcheroo", SWITCHEROO);
    let path = scenario
        .add_land_from_oracle(P0, "Homeward Path", HOMEWARD_PATH)
        .id();
    let mut runner = scenario.build();

    runner
        .cast(switcheroo)
        .target_objects(&[zidane, bear])
        .resolve();
    runner.advance_until_stack_empty();
    assert_eq!(controller(&runner, zidane), P1, "REACH: Zidane went to P1");
    assert_eq!(controller(&runner, bear), P0, "REACH: the bear came to P0");
    let (p0_before, p1_before) = (treasures(&runner, P0), treasures(&runner, P1));

    runner.activate(path, 1).resolve();
    runner.advance_until_stack_empty();
    assert_eq!(
        controller(&runner, zidane),
        P0,
        "REACH: Zidane returned to P0"
    );
    assert_eq!(
        controller(&runner, bear),
        P1,
        "REACH: the bear returned to P1"
    );
    assert_eq!(
        treasures(&runner, P1),
        p1_before + 1,
        "P1 lost Zidane to an opponent"
    );
    assert_eq!(
        treasures(&runner, P0),
        p0_before,
        "P0 didn't control Zidane when it lost the bear"
    );
}

/// Labelled synthetic two-instruction spell: two exchanges in one resolution,
/// one after the other (CR 608.2c).
const DOUBLE_SWITCHEROO: &str =
    "Exchange control of two target creatures. Exchange control of two target creatures.";

/// FAIL-CLOSED LIMIT, documented: a later instruction after Zidane moved.
/// The first exchange gives Zidane to P1 (Treasure for P0); the second moves
/// bear B from P1 to P2 while P1 controls Zidane, which by the rules is a
/// Treasure for P1. Collection can't order Zidane's change against the
/// second exchange, so that trigger doesn't fire. This pins only that nothing
/// fires for the wrong player: P0 gets exactly its own Treasure, and P2 none.
#[test]
fn limit_later_instruction_after_zidane_moved_never_fires_for_the_wrong_player() {
    let mut scenario = GameScenario::new_n_player(3, 42);
    scenario.at_phase(Phase::PreCombatMain);
    let zidane = add_zidane(&mut scenario, P0);
    let bear_a = scenario.add_vanilla(P1, 2, 2);
    let bear_b = scenario.add_vanilla(P1, 2, 2);
    let bear_c = scenario.add_vanilla(P2, 2, 2);
    let spell = add_free_spell(&mut scenario, P0, "Double Switcheroo", DOUBLE_SWITCHEROO);
    let mut runner = scenario.build();

    runner
        .cast(spell)
        .target_objects(&[zidane, bear_a, bear_b, bear_c])
        .resolve();
    runner.advance_until_stack_empty();
    assert_eq!(controller(&runner, zidane), P1, "REACH: Zidane went to P1");
    assert_eq!(controller(&runner, bear_a), P0, "REACH: bear A came to P0");
    assert_eq!(controller(&runner, bear_b), P2, "REACH: bear B went to P2");
    assert_eq!(controller(&runner, bear_c), P1, "REACH: bear C came to P1");
    assert_eq!(
        treasures(&runner, P0),
        1,
        "P0 gets its Treasure for Zidane and nothing for bear B"
    );
    assert_eq!(treasures(&runner, P2), 0);
}

/// Verbatim Oracle text (Scryfall).
const GRAB_THE_REINS: &str = "Choose one \u{2014}\n\u{2022} Until end of turn, you gain control \
    of target creature and it gains haste.\n\u{2022} Sacrifice a creature. Grab the Reins deals \
    damage equal to that creature's power to any target.\nEntwine {2}{R} (Choose both if you \
    pay the entwine cost.)";

/// FAIL-CLOSED LIMIT, documented: a permanent stolen and then sacrificed by
/// the same resolution. It was a permanent when it changed hands, so by the
/// rules P0 gets a Treasure; collection runs after it left, when it can't be
/// told apart from a spell that changed control on the stack, so it doesn't
/// fire. This pins only that nothing fires for the wrong player.
/// `p1_has_other` adds a second creature for P1, so the sacrifice pauses for
/// P1's choice instead of taking the only candidate.
#[test]
fn limit_steal_then_sacrifice_never_fires_for_the_wrong_player() {
    for p1_has_other in [false, true] {
        let mut scenario = GameScenario::new();
        scenario.at_phase(Phase::PreCombatMain);
        add_zidane(&mut scenario, P0);
        let bear = scenario.add_vanilla(P0, 2, 2);
        let other = p1_has_other.then(|| scenario.add_vanilla(P1, 1, 1));
        let reins = scenario
            .add_spell_to_hand_from_oracle(P1, "Grab the Reins", true, GRAB_THE_REINS)
            .from_oracle_text_with_keywords(&["Entwine"], GRAB_THE_REINS)
            .with_mana_cost(ManaCost::Cost {
                shards: vec![ManaCostShard::Red],
                generic: 3,
            })
            .id();
        scenario.with_mana_pool(
            P1,
            (0..7)
                .map(|_| ManaUnit::new(ManaType::Red, ObjectId(0), false, vec![]))
                .collect(),
        );
        let mut runner = scenario.build();
        give_priority(&mut runner, P1);

        runner
            .cast(reins)
            .modes(&[0, 1])
            .target_object(bear)
            .target_player(P0)
            .sacrifice_with(&[bear])
            .resolve();
        runner.advance_until_stack_empty();
        assert_eq!(
            runner.state().objects[&bear].zone,
            Zone::Graveyard,
            "REACH (p1_has_other {p1_has_other}): the stolen bear was sacrificed"
        );
        if let Some(other) = other {
            assert!(
                runner.state().battlefield.contains(&other),
                "REACH: P1 chose the stolen bear over its own creature"
            );
        }
        assert_eq!(
            treasures(&runner, P1),
            0,
            "p1_has_other {p1_has_other}: nothing fires for the thief"
        );
    }
}

/// Verbatim Oracle text (Scryfall).
const THRAGTUSK: &str = "When this creature enters, you gain 5 life.\nWhen this creature \
    leaves the battlefield, create a 3/3 green Beast creature token.";
/// Labelled synthetic steal-then-exile spell.
const STEAL_THEN_EXILE: &str = "Gain control of target creature until end of turn. Exile it.";

/// FAIL-CLOSED LIMIT, documented: steal-then-exile of a permanent with its own
/// leaves-the-battlefield trigger, so the exile's zone event is collected
/// separately from the control change. As in the sacrifice case, P0's
/// Treasure doesn't fire; this pins only that Thragtusk's trigger still
/// creates P1's Beast and nothing fires for the wrong player. `thragtusk`
/// false is the same steal-then-exile of a vanilla creature.
#[test]
fn limit_steal_then_exile_never_fires_for_the_wrong_player() {
    for thragtusk in [true, false] {
        let mut scenario = GameScenario::new();
        scenario.at_phase(Phase::PreCombatMain);
        add_zidane(&mut scenario, P0);
        let victim = if thragtusk {
            scenario
                .add_creature_from_oracle(P0, "Thragtusk", 5, 3, THRAGTUSK)
                .id()
        } else {
            scenario.add_vanilla(P0, 5, 3)
        };
        let spell = add_free_spell(&mut scenario, P1, "Steal Then Exile", STEAL_THEN_EXILE);
        let mut runner = scenario.build();
        give_priority(&mut runner, P1);

        runner.cast(spell).target_object(victim).resolve();
        runner.advance_until_stack_empty();
        assert_eq!(
            runner.state().objects[&victim].zone,
            Zone::Exile,
            "REACH (thragtusk {thragtusk}): the stolen creature was exiled"
        );
        let p1_beasts = runner
            .state()
            .battlefield
            .iter()
            .filter(|id| {
                let obj = &runner.state().objects[*id];
                obj.name.contains("Beast") && obj.controller == P1
            })
            .count();
        assert_eq!(
            p1_beasts,
            usize::from(thragtusk),
            "REACH: Thragtusk's leaves trigger is controlled by its last controller, P1"
        );
        assert_eq!(
            treasures(&runner, P1),
            0,
            "thragtusk {thragtusk}: nothing fires for the thief"
        );
    }
}
