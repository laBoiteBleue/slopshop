// A font style's name as the font pickers show it: its weight's (Photoshop's names), italic or
// not.
import { t } from "./i18n/index.svelte";
import { weightName } from "./typeText";

export function styleName(style: { weight: number; italic: boolean }): string {
  const name = t(`options.type.weight.${weightName(style.weight)}`);
  return style.italic ? t("options.type.italic", { style: name }) : name;
}
