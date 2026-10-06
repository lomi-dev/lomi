import type { PendingPermission } from "./types.ts";
interface Choice {
  label: string;
  value: unknown;
  allow: boolean;
}
const denied =
  /^(deny|denied|decline|cancel|reject|rejected|cancelled|canceled|abort)(?:_|$)/;
function object(value: unknown): Record<string, unknown> {
  return value && typeof value === "object"
    ? (value as Record<string, unknown>)
    : {};
}
export function permissionControls(permission: PendingPermission): {
  choices: Choice[];
  textInput?: { label: string; initial: string; multiline: boolean };
  cancel: Choice | undefined;
} {
  const raw = object(permission.permission.raw);
  const pi = raw.type === "extension_ui_request";
  const method = pi ? String(raw.method ?? "") : "";
  let choices: Choice[];
  if (pi && method === "confirm")
    choices = [
      { label: "Cancel", value: false, allow: false },
      { label: "Confirm", value: true, allow: true },
    ];
  else
    choices = permission.permission.choices.map((value) => {
      const item = object(value);
      const id =
        typeof value === "string"
          ? value
          : String(item.kind ?? item.id ?? item.optionId ?? "");
      return {
        label: String(item.name ?? item.label ?? (id || "Native choice")),
        value: typeof item.optionId === "string" ? item.optionId : value,
        allow:
          pi && method === "select"
            ? value !== null
            : typeof value === "boolean"
              ? value
              : !denied.test(id.toLowerCase()),
      };
    });
  if (pi && ["input", "editor", "select"].includes(method))
    choices.unshift({ label: "Cancel", value: null, allow: false });
  const cancel = choices.find((choice) => !choice.allow);
  const textInput =
    pi && ["input", "editor"].includes(method)
      ? {
          label: String(raw.title ?? raw.message ?? "Response"),
          initial: String(raw.initialValue ?? raw.prefill ?? ""),
          multiline: method === "editor",
        }
      : undefined;
  return { choices, textInput, cancel };
}
