//! CR 603.2 + CR 603.10d: "Whenever an opponent gains control of a permanent
//! from you" (Zidane, Tantalus Thief) — the opponent-gains half of the
//! look-back control-change triggers, driven through real control-changing
//! spells and the cleanup reversion.

use engine::game::scenario::{GameRunner, GameScenario, P0, P1};
use engine::types::format::FormatConfig;
use engine::types::game_state::WaitingFor;
use engine::types::identifiers::ObjectId;
use engine::types::mana::ManaCost;
use engine::types::phase::Phase;
use engine::types::player::PlayerId;

const P2: PlayerId = PlayerId(2);

/// Verbatim Oracle text (Scryfall).
const ZIDANE: &str = "When Zidane enters, gain control of target creature an opponent \
    controls until end of turn. Untap it. It gains lifelink and haste until end of turn.\n\
    Whenever an opponent gains control of a permanent from you, you create a Treasure token.";
const ACT_OF_TREASON: &str = "Gain control of target creature until end of turn. Untap that \
    creature. It gains haste until end of turn. (It can attack and {T} this turn.)";
const SUBJUGATE_THE_HOBBITS: &str =
    "Gain control of each noncommander creature with mana value 3 or less.";
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

fn add_zidane(scenario: &mut GameScenario, player: PlayerId) -> ObjectId {
    scenario
        .add_creature_from_oracle(player, "Zidane, Tantalus Thief", 3, 3, ZIDANE)
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

/// CR 603.2c: a mass steal is one occurrence per permanent, Zidane included.
#[test]
fn mass_steal_fires_once_per_permanent_including_zidane() {
    let mut scenario = GameScenario::new();
    scenario.at_phase(Phase::PreCombatMain);
    let zidane = add_zidane(&mut scenario, P0);
    let bear = scenario.add_vanilla(P0, 2, 2);
    let subjugate = add_free_spell(
        &mut scenario,
        P1,
        "Subjugate the Hobbits",
        SUBJUGATE_THE_HOBBITS,
    );
    let mut runner = scenario.build();
    give_priority(&mut runner, P1);

    runner.cast(subjugate).resolve();
    runner.advance_until_stack_empty();
    assert_eq!(controller(&runner, zidane), P1, "REACH: Zidane stolen");
    assert_eq!(controller(&runner, bear), P1, "REACH: bear stolen");
    assert_eq!(
        treasures(&runner, P0),
        2,
        "one Treasure per stolen permanent"
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

/// A steal between two other players is not "from you".
#[test]
fn steal_between_other_players_does_not_fire() {
    let mut scenario = GameScenario::new_n_player(3, 42);
    scenario.at_phase(Phase::PreCombatMain);
    add_zidane(&mut scenario, P0);
    let victim = scenario.add_vanilla(P2, 2, 2);
    let steal = add_free_spell(&mut scenario, P1, "Act of Treason", ACT_OF_TREASON);
    let mut runner = scenario.build();
    give_priority(&mut runner, P1);

    runner.cast(steal).target_object(victim).resolve();
    runner.advance_until_stack_empty();
    assert_eq!(
        controller(&runner, victim),
        P1,
        "REACH: P1 stole P2's creature"
    );
    assert_eq!(treasures(&runner, P0), 0);
    assert_eq!(treasures(&runner, P2), 0);
}
