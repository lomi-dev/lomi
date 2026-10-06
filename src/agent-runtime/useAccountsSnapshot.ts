import { useEffect, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { api, errorMessage } from "../api";
import type { AccountsSnapshot } from "./types";
export function useAccountsSnapshot() {
  const [snapshot, setSnapshot] = useState<AccountsSnapshot>();
  const [error, setError] = useState("");
  const [busy, setBusy] = useState(false);
  useEffect(() => {
    let live = true;
    const refresh = async () => {
      try {
        const next = await api<AccountsSnapshot>("agent_accounts_snapshot");
        if (live)
          setSnapshot((current) =>
            !current || next.revision >= current.revision ? next : current,
          );
      } catch (cause) {
        if (live) setError(errorMessage(cause));
      }
    };
    const subscription = listen("agent-runtime-changed", () => void refresh())
      .then((stop) => {
        if (!live) stop();
        else void refresh();
        return stop;
      })
      .catch((cause) => {
        if (live) setError(errorMessage(cause));
      });
    return () => {
      live = false;
      void subscription.then((stop) => stop?.());
    };
  }, []);
  async function command(name: string, request: Record<string, unknown>) {
    if (busy) throw new Error("An account operation is already in progress.");
    setBusy(true);
    setError("");
    try {
      const next = await api<AccountsSnapshot>(name, { request });
      setSnapshot((current) =>
        !current || next.revision >= current.revision ? next : current,
      );
      return next;
    } catch (cause) {
      setError(errorMessage(cause));
      throw cause;
    } finally {
      setBusy(false);
    }
  }
  return { snapshot, error, busy, command };
}
