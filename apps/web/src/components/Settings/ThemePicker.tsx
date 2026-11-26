import { readTheme, saveTheme, THEMES, type Theme } from "@web/theme";
import { useState } from "react";
import { LuMonitor, LuMoon, LuSun } from "react-icons/lu";

const LABELS: Record<Theme, string> = { system: "System", light: "Light", dark: "Dark" };
const ICONS: Record<Theme, typeof LuSun> = { system: LuMonitor, light: LuSun, dark: LuMoon };

/** Light, dark, or the system's scheme, as a radio group. */
export function ThemePicker() {
  const [theme, setTheme] = useState<Theme>(readTheme);

  return (
    <fieldset>
      <legend className="field-label">Theme</legend>
      <div className="mt-2 grid grid-cols-3 gap-2 sm:max-w-sm">
        {THEMES.map((option) => {
          const Icon = ICONS[option];
          return (
            <label key={option} className="choice-card flex-col items-center gap-1.5 py-3">
              <input
                type="radio"
                name="theme"
                value={option}
                checked={theme === option}
                onChange={() => {
                  saveTheme(option);
                  setTheme(option);
                }}
                className="sr-only"
              />
              <Icon aria-hidden className="size-5" />
              {LABELS[option]}
            </label>
          );
        })}
      </div>
    </fieldset>
  );
}
