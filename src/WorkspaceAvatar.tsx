import type { CSSProperties } from "react";
import { Bot, Briefcase, Rocket } from "lucide-react";
import { Code, Folder, Globe, Layers, Terminal } from "./icons";
import { msg } from "./i18n";
import type { WorkspaceAppearance } from "./workspace-appearance";

export const workspaceIconOptions = [
  { id: "layers", label: msg("Layers"), Icon: Layers },
  { id: "folder", label: msg("Folder"), Icon: Folder },
  { id: "code", label: msg("Code"), Icon: Code },
  { id: "terminal", label: msg("Terminal"), Icon: Terminal },
  { id: "globe", label: msg("Globe"), Icon: Globe },
  { id: "rocket", label: msg("Rocket"), Icon: Rocket },
  { id: "bot", label: msg("Bot"), Icon: Bot },
  { id: "briefcase", label: msg("Briefcase"), Icon: Briefcase },
];

export default function WorkspaceAvatar({
  appearance,
  className = "",
}: {
  appearance?: WorkspaceAppearance;
  className?: string;
}) {
  const icon = appearance?.icon || "layers";
  const Icon = workspaceIconOptions.find((option) => option.id === icon)?.Icon;
  const color = appearance?.color;
  const rgb = color
    ? [1, 3, 5].map(
        (start) => parseInt(color.slice(start, start + 2), 16) / 255,
      )
    : [];
  const luminance = rgb.reduce(
    (sum, value, index) =>
      sum +
      (value <= 0.04045 ? value / 12.92 : ((value + 0.055) / 1.055) ** 2.4) *
        [0.2126, 0.7152, 0.0722][index],
    0,
  );
  return (
    <span
      className={`workspace-avatar ${className}`}
      aria-hidden="true"
      style={
        color
          ? ({
              "--workspace-avatar-color": color,
              "--workspace-avatar-ink":
                luminance > 0.179 ? "#111111" : "#ffffff",
            } as CSSProperties)
          : undefined
      }
    >
      {appearance?.image ? (
        <img src={appearance.image} alt="" />
      ) : Icon ? (
        <Icon size={19} />
      ) : (
        <span className="workspace-avatar-emoji">{icon}</span>
      )}
    </span>
  );
}
