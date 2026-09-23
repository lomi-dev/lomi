import { useEffect, useRef, useState, type ReactNode } from "react";
import { api, errorMessage } from "./api";
import { Minus, Plus, RotateCcw, X } from "./icons";
import type { FileTab } from "./model";
import { imagePreviewType } from "./image-preview";
import ResourceIcon from "./ResourceIcon";
import { IconButton } from "./ui";
import { useAgentPreview } from "./agent-preview";

export default function ImagePreview({
  tab,
  active = true,
  onClose,
  sourceText,
  children,
}: {
  tab: FileTab;
  active?: boolean;
  onClose?: () => void;
  sourceText?: string;
  children?: ReactNode;
}) {
  const prepared = useAgentPreview(tab.id)?.image;
  const [source, setSource] = useState("");
  const [error, setError] = useState("");
  const [attempt, setAttempt] = useState(0);
  const [dimensions, setDimensions] = useState<{
    width: number;
    height: number;
  }>();
  const [zoom, setZoom] = useState<number | "fit">("fit");
  const viewport = useRef<HTMLDivElement>(null);
  const image = useRef<HTMLImageElement>(null);

  useEffect(() => {
    let current = true;
    const reader = new FileReader();
    setSource("");
    setError("");
    setDimensions(undefined);
    reader.onload = () => {
      if (current) setSource(String(reader.result));
    };
    reader.onerror = () => {
      if (current)
        setError("Cannot read this image. Try reloading it from disk.");
    };
    if (sourceText === undefined && tab.agentPreview) {
      if (prepared)
        setSource(`data:${prepared.mimeType};base64,${prepared.dataBase64}`);
      else
        setError(
          "Reopen this preview through Agent control to load the image.",
        );
    } else if (sourceText !== undefined) {
      reader.readAsDataURL(
        new Blob([sourceText], { type: "image/svg+xml;charset=utf-8" }),
      );
    } else
      void api<ArrayBuffer>("read_image_file", {
        root: tab.root,
        relative: tab.relative,
      })
        .then((bytes) => {
          if (current)
            reader.readAsDataURL(
              new Blob([bytes], { type: imagePreviewType(tab.relative) }),
            );
        })
        .catch((error) => {
          if (current) setError(errorMessage(error));
        });
    return () => {
      current = false;
      reader.abort();
    };
  }, [tab.root, tab.relative, tab.agentPreview, prepared, sourceText, attempt]);

  useEffect(() => {
    if (
      active &&
      !viewport.current?.parentElement?.contains(document.activeElement)
    )
      viewport.current?.focus({ preventScroll: true });
  }, [active]);

  const changeZoom = (factor: number) => {
    const current =
      zoom === "fit"
        ? (image.current?.width ?? 0) / (dimensions?.width ?? 1)
        : zoom;
    setZoom(
      Math.min(
        8,
        Math.max(
          Math.min(current, 0.1),
          Math.round(current * factor * 100) / 100,
        ),
      ),
    );
  };
  const ready = !!dimensions && !error;

  return (
    <section
      className="file-editor image-preview"
      aria-label={`Image preview for ${tab.title}`}
    >
      <header className="editor-heading" data-pane-drag-handle>
        {sourceText === undefined && (
          <>
            <ResourceIcon path={`${tab.root}/${tab.relative}`} size={15} />
            <span className="editor-path" title={`${tab.root}/${tab.relative}`}>
              {tab.relative}
            </span>
          </>
        )}
        <div className="editor-actions">
          <IconButton
            title="Zoom out"
            disabled={!ready || (typeof zoom === "number" && zoom <= 0.1)}
            onClick={() => changeZoom(0.8)}
          >
            <Minus size={15} />
          </IconButton>
          <button
            className="button"
            title={prepared ? "Preview pixels" : "Actual size"}
            disabled={!ready}
            aria-pressed={zoom === 1}
            onClick={() => setZoom(1)}
          >
            100%
          </button>
          <IconButton
            title="Zoom in"
            disabled={!ready || zoom === 8}
            onClick={() => changeZoom(1.25)}
          >
            <Plus size={15} />
          </IconButton>
          <button
            className="button"
            title="Fit image to panel"
            disabled={!ready}
            aria-pressed={zoom === "fit"}
            onClick={() => setZoom("fit")}
          >
            Fit
          </button>
          {sourceText === undefined && !tab.agentPreview && (
            <IconButton
              title="Reload image from disk"
              onClick={() => setAttempt((value) => value + 1)}
            >
              <RotateCcw size={15} />
            </IconButton>
          )}
          {onClose && (
            <IconButton title="Close panel" onClick={onClose}>
              <X size={15} />
            </IconButton>
          )}
        </div>
      </header>
      <div className="editor-content">
        <div
          className="image-viewport"
          ref={viewport}
          tabIndex={0}
          aria-label={`View ${tab.title}`}
        >
          {error ? (
            <div className="empty-message" role="alert">
              <strong>Cannot display image</strong>
              <p>{error}</p>
              <button
                className="button"
                onClick={() => setAttempt((value) => value + 1)}
              >
                Try again
              </button>
            </div>
          ) : (
            <>
              {!ready && (
                <div className="empty-message image-loading" role="status">
                  Opening image…
                </div>
              )}
              {source && (
                <div
                  className={`image-canvas${zoom === "fit" ? " is-fit" : ""}`}
                >
                  <img
                    ref={image}
                    src={source}
                    alt={tab.relative}
                    draggable={false}
                    style={{
                      visibility: ready ? "visible" : "hidden",
                      width:
                        zoom === "fit"
                          ? undefined
                          : (dimensions?.width ?? 0) * zoom,
                    }}
                    onLoad={(event) =>
                      setDimensions({
                        width: event.currentTarget.naturalWidth,
                        height: event.currentTarget.naturalHeight,
                      })
                    }
                    onError={() =>
                      setError(
                        "The file is damaged or its image format is not supported by this system.",
                      )
                    }
                  />
                </div>
              )}
            </>
          )}
        </div>
        {children}
      </div>
    </section>
  );
}
