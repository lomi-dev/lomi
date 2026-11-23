// Labels are existing i18n keys (see AgentControlPairing); callers translate them.
const accessAreas = [
  { label: "Terminal", prefixes: ["terminal."] },
  {
    label: "Files & editor",
    prefixes: ["files.", "editor.", "artifact."],
  },
  { label: "Git", prefixes: ["git."] },
  { label: "Browser", prefixes: ["browser."] },
  { label: "Android", prefixes: ["android."] },
  {
    label: "Workspaces & panels",
    prefixes: ["workspace.write", "workspace.close", "project.", "panel."],
  },
  { label: "Application preferences", prefixes: ["settings."] },
];
// Read-only scopes; anything else in an area lets the agent change state.
const readScopes = new Set([
  "files.read",
  "editor.read",
  "git.read",
  "browser.read",
  "browser.navigate",
  "browser.capture_composite",
  "android.read",
  "android.observe",
  "android.logs",
  "android.capture",
  "terminal.read",
  "settings.read",
  "settings.open",
  "panel.focus",
]);

/** Groups raw grant scopes into the permission areas shown when pairing. */
export function sessionAccess(scopes: string[]) {
  return accessAreas.flatMap(({ label, prefixes }) => {
    const matched = scopes.filter((scope) =>
      prefixes.some((prefix) => scope.startsWith(prefix)),
    );
    return matched.length
      ? [
          {
            label,
            scopes: matched,
            changes: matched.some((scope) => !readScopes.has(scope)),
          },
        ]
      : [];
  });
}
