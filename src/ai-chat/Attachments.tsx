import { useEffect, useState } from "react";
import { useT } from "../i18n";
import { File, FileText, X } from "../icons";
import { IconButton, Modal } from "../ui";
import { formatSize } from "./attachment-size";
import { attachmentPreview } from "./attachment-store";
import type { Attachment } from "./types";

function usePreview(attachment: Attachment) {
  const [url, setUrl] = useState<string>();
  useEffect(() => {
    setUrl(undefined);
    if (attachment.kind !== "image") return;
    let current = true;
    attachmentPreview(attachment.id).then(
      (value) => current && setUrl(value),
      () => {},
    );
    return () => {
      current = false;
    };
  }, [attachment.id, attachment.kind]);
  return url;
}

const KindIcon = ({ kind }: { kind: Attachment["kind"] }) =>
  kind === "text" ? <FileText size={16} /> : <File size={16} />;

function Chip({
  attachment,
  onRemove,
}: {
  attachment: Attachment;
  onRemove?: () => void;
}) {
  const { t } = useT();
  const url = usePreview(attachment);
  return (
    <li className="thread-chip">
      <span className="thread-chip-thumb">
        {url ? <img src={url} alt="" /> : <KindIcon kind={attachment.kind} />}
      </span>
      <span className="thread-chip-name" title={attachment.name}>
        {attachment.name}
      </span>
      <span className="thread-chip-size">{formatSize(attachment.size)}</span>
      {onRemove && (
        <IconButton
          title={t("Remove {name}", { name: attachment.name })}
          className="thread-chip-remove"
          onClick={onRemove}
        >
          <X size={12} />
        </IconButton>
      )}
    </li>
  );
}

export function PendingAttachments({
  items,
  importing,
  error,
  onRemove,
}: {
  items: readonly Attachment[];
  importing: number;
  error: string;
  onRemove: (id: string) => void;
}) {
  const { t } = useT();
  if (!items.length && !importing && !error) return null;
  return (
    <div className="thread-attachments">
      {(items.length > 0 || importing > 0) && (
        <ul className="thread-chips">
          {items.map((item) => (
            <Chip
              key={item.id}
              attachment={item}
              onRemove={() => onRemove(item.id)}
            />
          ))}
          {importing > 0 && (
            <li className="thread-chip thread-chip-busy" role="status">
              <span className="thread-chip-thumb" aria-hidden="true" />
              <span className="thread-chip-name">{t("Importing…")}</span>
            </li>
          )}
        </ul>
      )}
      {error && (
        <p className="thread-attachment-error" role="alert">
          {error}
        </p>
      )}
    </div>
  );
}

function Thumb({ attachment }: { attachment: Attachment }) {
  const { t } = useT();
  const url = usePreview(attachment);
  const [open, setOpen] = useState(false);
  return (
    <>
      <button
        type="button"
        className="thread-image"
        title={attachment.name}
        aria-label={t("Open {name}", { name: attachment.name })}
        onClick={() => url && setOpen(true)}
      >
        {url ? <img src={url} alt={attachment.name} /> : <File size={18} />}
      </button>
      {open && url && (
        <Modal
          title={attachment.name}
          wide
          className="thread-lightbox"
          onClose={() => setOpen(false)}
        >
          <img src={url} alt={attachment.name} />
        </Modal>
      )}
    </>
  );
}

export function MessageAttachments({
  attachments,
}: {
  attachments: readonly Attachment[];
}) {
  if (!attachments.length) return null;
  const images = attachments.filter((item) => item.kind === "image");
  const files = attachments.filter((item) => item.kind !== "image");
  return (
    <div className="thread-message-attachments">
      {images.length > 0 && (
        <div className="thread-images">
          {images.map((item) => (
            <Thumb key={item.id} attachment={item} />
          ))}
        </div>
      )}
      {files.length > 0 && (
        <ul className="thread-chips">
          {files.map((item) => (
            <Chip key={item.id} attachment={item} />
          ))}
        </ul>
      )}
    </div>
  );
}
