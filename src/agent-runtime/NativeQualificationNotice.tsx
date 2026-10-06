import type { CliAgent } from "../cli-agents";

export default function NativeQualificationNotice({
  qualified,
  cli,
}: {
  qualified?: boolean;
  cli?: CliAgent | null;
}) {
  if (qualified) return null;
  return (
    <div role="note" aria-label="CLI background work limitation">
      <strong>CLI background work limitation</strong>
      {cli === "codex" || cli === "claude" ? (
        <p>
          In this restricted mode, account switching is unavailable while work
          is running. Completed local work can release its account. Unknown tool
          or remote effects keep the task and account protected; Lomi will show
          the required recovery steps. Acknowledging effects alone cannot
          release the lock.
        </p>
      ) : (
        <p>
          This CLI’s hooks and helper processes cannot yet be verified as
          stopped, even after a text-only run. Continuing the task or releasing
          its project and account requires restarting your computer, having Lomi
          verify the restart, and explicitly recovering the task. Acknowledging
          effects alone cannot release the lock. Task and app close, updates,
          and app restarts remain blocked while this ownership is unresolved.
        </p>
      )}
      {cli === "pi" && <p>Managed Pi runs with extensions disabled.</p>}
    </div>
  );
}
