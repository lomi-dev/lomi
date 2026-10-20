/** Only the immutable built-in Lomi shell can replace Linux WebView alpha. */
export function opaqueWindowBackground(): [number, number, number] | null {
  if (document.documentElement.dataset.theme !== "lomi") return null;
  const shell = document.querySelector<HTMLElement>(".app-shell");
  if (!shell || !shell.isConnected || innerWidth <= 0 || innerHeight <= 0)
    return null;
  for (
    let element: HTMLElement | null = shell;
    element;
    element = element.parentElement
  ) {
    const bounds = element.getBoundingClientRect();
    if (
      bounds.left > 0 ||
      bounds.top > 0 ||
      bounds.right < innerWidth ||
      bounds.bottom < innerHeight
    )
      return null;
    const style = getComputedStyle(element);
    if (style.opacity !== "1" || style.visibility !== "visible") return null;
    for (const property of [
      "border-top-left-radius",
      "border-top-right-radius",
      "border-bottom-left-radius",
      "border-bottom-right-radius",
      "border-top-width",
      "border-right-width",
      "border-bottom-width",
      "border-left-width",
    ]) {
      if (
        !/^0(?:px|%)?(?:\s+0(?:px|%)?)?$/.test(style.getPropertyValue(property))
      )
        return null;
    }
    for (const property of [
      "mask-image",
      "-webkit-mask-image",
      "-webkit-mask-box-image-source",
      "clip-path",
      "filter",
      "backdrop-filter",
      "-webkit-backdrop-filter",
      "transform",
      "translate",
      "rotate",
      "scale",
      "perspective",
      "animation-name",
    ]) {
      const value = style.getPropertyValue(property);
      if (value && value !== "none") return null;
    }
    if (
      style.mixBlendMode !== "normal" ||
      style.clip !== "auto" ||
      style.transitionDuration
        .split(",")
        .some((duration) => !/^0(?:s|ms)$/.test(duration.trim()))
    )
      return null;
  }
  try {
    const canvas = document.createElement("canvas");
    canvas.width = canvas.height = 1;
    const context = canvas.getContext("2d", { willReadFrequently: true });
    if (!context) return null;
    const color = getComputedStyle(shell).backgroundColor;
    // Canvas rounds alpha to eight bits. A nearly opaque rgba() must not be
    // promoted to opaque; the built-in palette uses plain sRGB rgb() colors.
    if (!/^rgb\(\s*\d+\s*,\s*\d+\s*,\s*\d+\s*\)$/.test(color)) return null;
    // An unsupported canvas color must not silently retain an opaque default.
    context.fillStyle = "#010203";
    context.fillStyle = color;
    const first = context.fillStyle;
    context.fillStyle = "#040506";
    context.fillStyle = color;
    if (context.fillStyle !== first) return null;
    context.fillRect(0, 0, 1, 1);
    const [red, green, blue, alpha] = context.getImageData(0, 0, 1, 1).data;
    return alpha === 255 ? [red, green, blue] : null;
  } catch {
    return null;
  }
}
