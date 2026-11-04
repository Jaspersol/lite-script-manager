import { common } from "./en/common";
import { backend } from "./en/backend";
import { app } from "./en/app";
import { panel } from "./en/panel";
import { history } from "./en/history";
import { schedules } from "./en/schedules";
import { triggers } from "./en/triggers";
import { settings } from "./en/settings";
import { hint } from "./en/hint";

/** Chinese source string -> English text. Placeholders use `{name}`. */
export const en: Record<string, string> = {
  ...common,
  ...backend,
  ...app,
  ...panel,
  ...history,
  ...schedules,
  ...triggers,
  ...settings,
  ...hint,
};
