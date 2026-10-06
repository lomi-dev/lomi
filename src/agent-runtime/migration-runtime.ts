type Release = () => Promise<void>;
let acquire: (() => Promise<Release>) | undefined;
export function configureSessionPublication(next: () => Promise<Release>) {
  acquire = next;
  return () => {
    if (acquire === next) acquire = undefined;
  };
}
export async function acquireSessionPublication() {
  if (!acquire)
    throw new Error(
      "Open the main workspace before importing saved CLI history.",
    );
  return acquire();
}
