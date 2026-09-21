import { useEffect, useState } from "react";

export function useTerminalTitleReveal(enabled: boolean) {
  const [revealed, setRevealed] = useState(false);

  useEffect(() => {
    setRevealed(false);
    if (!enabled) return;
    let pressedAt: number | undefined;
    let timeout: ReturnType<typeof setTimeout> | undefined;
    const reset = () => {
      clearTimeout(timeout);
      pressedAt = undefined;
      setRevealed(false);
    };
    const keydown = (event: KeyboardEvent) => {
      if (event.key !== "Control" || event.repeat || pressedAt !== undefined)
        return;
      clearTimeout(timeout);
      pressedAt = performance.now();
      setRevealed(true);
      timeout = setTimeout(() => {
        if (pressedAt === undefined) setRevealed(false);
      }, 5000);
    };
    const keyup = (event: KeyboardEvent) => {
      if (event.key !== "Control" || event.ctrlKey || pressedAt === undefined)
        return;
      const held = performance.now() - pressedAt;
      pressedAt = undefined;
      if (held > 1000) reset();
    };
    const visibility = () => {
      if (document.hidden) reset();
    };
    // Observe Control without consuming shortcuts or terminal input.
    window.addEventListener("keydown", keydown, true);
    window.addEventListener("keyup", keyup, true);
    window.addEventListener("blur", reset);
    document.addEventListener("visibilitychange", visibility);
    return () => {
      clearTimeout(timeout);
      window.removeEventListener("keydown", keydown, true);
      window.removeEventListener("keyup", keyup, true);
      window.removeEventListener("blur", reset);
      document.removeEventListener("visibilitychange", visibility);
    };
  }, [enabled]);

  return enabled && revealed;
}
