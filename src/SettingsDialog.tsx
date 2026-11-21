import { useT } from "./i18n";
import { useLayoutEffect, useRef, type ReactNode } from "react";

// Settings render as a modal sheet over the workbench. The element stays
// mounted while closed so pages keep their drafts between openings, and an
// open <dialog> also makes native browser views step aside.
export default function SettingsDialog({
  open,
  onClose,
  children,
}: {
  open: boolean;
  onClose: () => void;
  children: ReactNode;
}) {
  const { t } = useT();
  const dialog = useRef<HTMLDialogElement>(null);
  useLayoutEffect(() => {
    const element = dialog.current;
    if (!element) return;
    if (open && !element.open) element.showModal();
    if (!open && element.open) element.close();
  }, [open]);
  return (
    <dialog
      ref={dialog}
      className="settings-dialog"
      aria-label={t("Settings")}
      onCancel={(event) => {
        event.preventDefault();
        onClose();
      }}
      onClick={(event) => {
        if (event.target === event.currentTarget) onClose();
      }}
    >
      {children}
    </dialog>
  );
}
