/** Extracts the owner from a github.com https or ssh remote URL. */
export function githubOwner(remote: string): string | null {
  const match =
    /^(?:https?:\/\/(?:[^/@\s]+@)?|ssh:\/\/git@|git@)github\.com[/:]([A-Za-z0-9](?:[A-Za-z0-9-]{0,38}))\/[^/\s]+?(?:\.git)?\/?$/i.exec(
      remote.trim(),
    );
  return match ? match[1] : null;
}
