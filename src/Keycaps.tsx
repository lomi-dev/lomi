import { keyNames } from "@lomi-dev/plugin-sdk/shortcuts";
import { macOS } from "./api";

const MAC_SYMBOLS: Record<string, string> = {
  Meta: "⌘",
  Ctrl: "⌃",
  Alt: "⌥",
  Shift: "⇧",
};

function keyLabel(part: string) {
  if (macOS && MAC_SYMBOLS[part]) return MAC_SYMBOLS[part];
  if (part === "Meta") return "Win";
  return keyNames[part] ?? part.replace(/^(Key|Digit)/, "");
}

export default function Keycaps({
  binding,
  className,
}: {
  binding: string | null;
  className?: string;
}) {
  if (!binding) return null;
  return (
    <span className={className ? `keycaps ${className}` : "keycaps"}>
      {binding.split("+").map((part, index) => (
        <kbd key={index}>{keyLabel(part)}</kbd>
      ))}
    </span>
  );
}
