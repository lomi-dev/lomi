export const WORKSPACE_ICONS = [
  "layers",
  "folder",
  "code",
  "terminal",
  "globe",
  "rocket",
  "bot",
  "briefcase",
] as const;
export const MAX_WORKSPACE_ICON_LENGTH = 16;
export const MAX_WORKSPACE_IMAGE_BYTES = 100 * 1024;
export const MAX_WORKSPACE_IMAGE_SIZE = 128;

export interface WorkspaceAppearance {
  icon?: string;
  color?: string;
  image?: string;
  /** Id of a picked file in the native workspace-logos store. */
  logo?: string;
}

export function sanitizeWorkspaceAppearance(
  value: unknown,
): WorkspaceAppearance | undefined {
  if (!value || typeof value !== "object" || Array.isArray(value))
    return undefined;
  const source = value as Record<string, unknown>;
  const result: WorkspaceAppearance = {};
  if (typeof source.icon === "string") {
    const icon = source.icon.trim();
    if (
      icon &&
      Array.from(icon).length <= MAX_WORKSPACE_ICON_LENGTH &&
      !/[\p{Cc}\p{Cs}]/u.test(icon)
    )
      result.icon = icon;
  }
  if (typeof source.color === "string" && /^#[\da-f]{6}$/i.test(source.color))
    result.color = source.color;
  if (
    typeof source.logo === "string" &&
    /^[0-9a-f]{32}\.(?:png|jpg|svg|webp|ico)$/.test(source.logo)
  )
    result.logo = source.logo;
  if (
    typeof source.image === "string" &&
    source.image.length <= Math.ceil(MAX_WORKSPACE_IMAGE_BYTES / 3) * 4 + 32
  ) {
    const match =
      /^data:image\/(?:png|jpeg|webp);base64,([A-Za-z0-9+/]+={0,2})$/.exec(
        source.image,
      );
    if (match && match[1].length % 4 === 0) {
      const padding = match[1].endsWith("==")
        ? 2
        : match[1].endsWith("=")
          ? 1
          : 0;
      if ((match[1].length / 4) * 3 - padding <= MAX_WORKSPACE_IMAGE_BYTES)
        result.image = source.image;
    }
  }
  return Object.keys(result).length ? result : undefined;
}
