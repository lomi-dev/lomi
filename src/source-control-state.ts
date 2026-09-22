interface RepositoryState {
  message: string;
  busy: boolean;
  remoteStatus: string;
  historyRevision: number;
  changesCollapsed: boolean;
}
const empty: RepositoryState = {
  message: "",
  busy: false,
  remoteStatus: "",
  historyRevision: 0,
  changesCollapsed: false,
};

// Owned by the workbench so panel and project switches preserve pending work.
export class SourceControlState {
  private repositories = new Map<string, RepositoryState>();
  private selected = new Map<string, string | null>();
  private listeners = new Set<() => void>();
  private revision = 0;
  subscribe = (listener: () => void) => {
    this.listeners.add(listener);
    return () => {
      this.listeners.delete(listener);
    };
  };
  snapshot = () => this.revision;
  private emit() {
    this.revision++;
    for (const listener of this.listeners) listener();
  }
  repository(root: string) {
    return this.repositories.get(root) ?? empty;
  }
  selection(project: string) {
    return this.selected.get(project) ?? null;
  }
  select(project: string, root: string | null) {
    this.selected.set(project, root);
    this.emit();
  }
  update(root: string, patch: Partial<RepositoryState>) {
    this.repositories.set(root, { ...this.repository(root), ...patch });
    this.emit();
  }
  async run(root: string, action: () => Promise<void>, progress = "") {
    if (this.repository(root).busy)
      throw new Error("Wait for the current Git operation to finish.");
    this.update(root, { busy: true, remoteStatus: progress });
    try {
      await action();
    } catch (error) {
      this.update(root, { remoteStatus: "" });
      throw error;
    } finally {
      this.update(root, {
        busy: false,
        historyRevision: this.repository(root).historyRevision + 1,
      });
    }
  }
  clearSubmittedMessage(root: string, submitted: string) {
    if (this.repository(root).message === submitted)
      this.update(root, { message: "" });
  }
}
