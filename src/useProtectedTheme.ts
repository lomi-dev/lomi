import { useLayoutEffect } from "react";
import { protectThemeControls } from "./theme/runtime";

export function useProtectedTheme(active: boolean) {
  useLayoutEffect(
    () => (active ? protectThemeControls() : undefined),
    [active],
  );
}
