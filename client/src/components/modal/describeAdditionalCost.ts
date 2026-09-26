import type { TFunction } from "i18next";

import type { SerializedAbilityCost } from "../../adapter/types.ts";

/**
 * CR 601.2f-h: Compact display copy for the non-mana portion of an
 * alternative cost (Solitude's Evoke "Exile a white card from your hand",
 * Sabin's Blitz "Discard a card"). Mirrors the engine's typed `AbilityCost`
 * taxonomy 1:1 by the discriminant `type` field — the FE does not interpret
 * game state, it just renders the engine-provided variant.
 */
export function describeAdditionalCost(
  cost: SerializedAbilityCost,
  t: TFunction<"game">,
): string {
  switch (cost.type) {
    case "Exile":
      return t("alternativeCost.additionalExile");
    case "Sacrifice":
      return t("alternativeCost.additionalSacrifice");
    case "PayLife":
      return t("alternativeCost.additionalPayLife");
    case "Discard":
      return t("alternativeCost.additionalDiscard");
    case "TapCreatures":
      return t("alternativeCost.additionalTapCreatures");
    default:
      return t("alternativeCost.additionalGeneric", { type: cost.type });
  }
}
