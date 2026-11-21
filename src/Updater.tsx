import { useCallback, useEffect, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { writeText } from "@tauri-apps/plugin-clipboard-manager";
import { openUrl } from "@tauri-apps/plugin-opener";
import { Update } from "@tauri-apps/plugin-updater";
import { api, errorMessage, native } from "./api";
import { Modal } from "./ui";
import type { ReleaseClosePreparation } from "./application-close";
import { useT } from "./i18n";

const releases = "https://github.com/lomi-dev/lomi/releases/latest";
type Status =
  | "idle"
  | "checking"
  | "available"
  | "downloading"
  | "preparing"
  | "installing"
  | "installed"
  | "error";

export function useUpdater(
  ready: boolean,
  beforeInstall: () => Promise<ReleaseClosePreparation | null>,
) {
  const { t } = useT();
  const [status, setStatus] = useState<Status>("idle");
  const [open, setOpen] = useState(false);
  const [update, setUpdate] = useState<Update | null>(null);
  const [instruction, setInstruction] = useState<string | null>(null);
  const [error, setError] = useState("");
  const [copied, setCopied] = useState(false);
  const [progress, setProgress] = useState<number>();
  const busy = useRef(false);
  const checking = useRef(false);
  const resource = useRef<Update | null>(null);
  const downloaded = useRef(false);
  const installed = useRef(false);
  const sequence = useRef(0);

  const runCheck = useCallback(async (manual: boolean) => {
    if (manual) setOpen(true);
    if (checking.current || busy.current || resource.current) return;
    checking.current = true;
    const seq = ++sequence.current;
    setStatus("checking");
    setError("");
    let found: Update | null = null;
    try {
      const environment = await api<{ linuxInstruction: string | null }>(
        "update_environment",
      );
      const metadata = await api<
        ConstructorParameters<typeof Update>[0] | null
      >("check_app_update");
      found = metadata ? new Update(metadata) : null;
      if (seq !== sequence.current) {
        await found?.close();
        return;
      }
      setInstruction(environment.linuxInstruction);
      resource.current = found;
      setUpdate(found);
      setStatus(found ? "available" : "idle");
      if (found) setOpen(true);
    } catch (error) {
      if (seq !== sequence.current) return;
      setError(errorMessage(error));
      setStatus("error");
    } finally {
      if (seq === sequence.current) checking.current = false;
    }
  }, []);

  useEffect(() => {
    if (!native || !ready) return;
    const timer = setTimeout(() => void runCheck(false), 3000);
    const unlisten = listen("check-for-updates", () => void runCheck(true));
    void unlisten.catch((error) => setError(errorMessage(error)));
    return () => {
      clearTimeout(timer);
      sequence.current++;
      checking.current = false;
      void unlisten.then((stop) => stop()).catch(() => {});
      void resource.current?.close().catch(() => {});
      resource.current = null;
    };
  }, [ready, runCheck]);

  const install = async () => {
    if (!update || instruction !== null || busy.current) return;
    busy.current = true;
    setError("");
    let restartRequested = false;
    let release: ReleaseClosePreparation | null = null;
    try {
      if (!installed.current && !downloaded.current) {
        setStatus("downloading");
        setProgress(undefined);
        let total: number | undefined;
        let received = 0;
        // Native connect/read timeouts bound stalls without limiting total time.
        await update.download((event) => {
          if (event.event === "Started") total = event.data.contentLength;
          if (event.event === "Progress") received += event.data.chunkLength;
          if (total)
            setProgress(Math.min(100, Math.round((received / total) * 100)));
        });
        downloaded.current = true;
      }
      setStatus("preparing");
      release = await beforeInstall();
      if (!release) {
        setStatus("available");
        return;
      }
      // Windows exits inside install(); persist and confirm closure beforehand.
      // Restart retries must also guard edits made after a failed restart.
      await api("reset_terminals");
      if (!installed.current) {
        setStatus("installing");
        await update.install();
        installed.current = true;
      }
      setStatus("installed");
      await api("restart_after_update");
      restartRequested = true;
    } catch (error) {
      setError(
        downloaded.current
          ? errorMessage(error)
          : t(
              "Could not download or verify the update: {error}. Try again or use GitHub Releases.",
              { error: errorMessage(error) },
            ),
      );
      setStatus("error");
    } finally {
      if (!restartRequested && release) {
        await release().catch((error) => setError(errorMessage(error)));
      }
      busy.current = restartRequested;
    }
  };

  const close = () => {
    if (busy.current) return;
    setOpen(false);
    setCopied(false);
    if (resource.current && !installed.current) {
      void resource.current.close().catch(() => {});
      resource.current = null;
      setUpdate(null);
      downloaded.current = false;
    }
  };
  const working = [
    "downloading",
    "preparing",
    "installing",
    "installed",
  ].includes(status);
  return {
    busy,
    dialog: open && (
      <Modal
        title={t("Software update")}
        className="updater-dialog"
        onClose={close}
        closeDisabled={busy.current}
      >
        <div className="dialog-form dialog-layout">
          <div className="dialog-content update-content">
            {update && (
              <p>
                Lomi {update.currentVersion} → {update.version}
              </p>
            )}
            {error && (
              <p role="alert">
                {installed.current
                  ? t("The update is installed. Restart Lomi to finish.")
                  : t("Update failed:")}{" "}
                {error}
              </p>
            )}
            {update?.body && <pre className="update-notes">{update.body}</pre>}
            {instruction && update && (
              <>
                <p>{t("Update Lomi outside the app:")}</p>
                <pre className="update-instruction">{instruction}</pre>
                <button
                  className="button"
                  onClick={() => {
                    void writeText(instruction)
                      .then(() => setCopied(true))
                      .catch((error) => setError(errorMessage(error)));
                  }}
                >
                  {copied ? t("Copied") : t("Copy instructions")}
                </button>
              </>
            )}
            {update && !instruction && !working && (
              <p>
                {t(
                  "The update will restart Lomi and end terminal sessions. You can save unsaved files before installation.",
                )}
              </p>
            )}
            <div role="status">
              {status === "checking" && t("Checking for updates…")}
              {status === "idle" && t("Lomi is up to date.")}
              {status === "downloading" && (
                <>
                  {t("Downloading update…")}{" "}
                  {progress === undefined ? "" : `${progress}%`}
                  <progress
                    aria-label={t("Download progress")}
                    max={100}
                    value={progress}
                  />
                </>
              )}
              {status === "preparing" && t("Preparing to close the workspace…")}
              {status === "installing" && t("Installing update…")}
              {status === "installed" &&
                t("Update installed. Restarting Lomi…")}
            </div>
          </div>
          <div className="dialog-actions">
            {!working && (
              <button className="button" onClick={close}>
                {update && !instruction ? t("Later") : t("Close")}
              </button>
            )}
            {!working &&
              update &&
              (instruction || (status === "error" && !installed.current)) && (
                <button
                  className={instruction ? "button button-primary" : "button"}
                  onClick={() =>
                    void openUrl(releases).catch((error) =>
                      setError(errorMessage(error)),
                    )
                  }
                >
                  {t("GitHub Releases")}
                </button>
              )}
            {!working && update && !instruction && (
              <button
                className="button button-primary"
                onClick={() => void install()}
              >
                {installed.current ? t("Restart now") : t("Update now")}
              </button>
            )}
            {status === "error" && !update && (
              <button
                className="button button-primary"
                onClick={() => void runCheck(true)}
              >
                {t("Try again")}
              </button>
            )}
          </div>
        </div>
      </Modal>
    ),
  };
}
