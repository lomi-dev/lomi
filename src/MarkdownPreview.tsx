import {
  memo,
  useDeferredValue,
  useEffect,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
  useSyncExternalStore,
} from "react";
import Markdown from "react-markdown";
import type { Components } from "react-markdown";
import remarkGfm from "remark-gfm";
import { openUrl } from "@tauri-apps/plugin-opener";
import { api, errorMessage } from "./api";
import type { EditorDocument } from "./editor-runtime";
import { markdownTarget } from "./markdown";
import { readAgentPreviewAsset } from "./agent-preview";

const plugins = [remarkGfm];

export default memo(function MarkdownPreview({
  document,
  onOpenFile,
  assetPermit,
}: {
  document: EditorDocument;
  onOpenFile: (root: string, relative: string) => void;
  assetPermit?: string | null;
}) {
  const current = useSyncExternalStore(
    document.subscribeText,
    document.getTextSnapshot,
  );
  const deferred = useDeferredValue(current);
  const text = useMemo(() => deferred.toString(), [deferred]);
  const content = useRef<HTMLElement>(null);
  const { root, relative } = document.location;
  useLayoutEffect(() => {
    const used = new Set<string>();
    content.current
      ?.querySelectorAll("h1, h2, h3, h4, h5, h6")
      .forEach((heading) => {
        if (heading.id && !heading.hasAttribute("data-markdown-heading")) {
          used.add(heading.id);
          return;
        }
        const base =
          (heading.textContent ?? "")
            .toLowerCase()
            .trim()
            .replace(/[^\p{L}\p{N}\s_-]/gu, "")
            .replace(/\s+/g, "-") || "section";
        let id = base;
        for (let index = 1; used.has(id); index++) id = `${base}-${index}`;
        used.add(id);
        heading.id = id;
        heading.setAttribute("data-markdown-heading", "");
      });
  }, [text]);
  const components = useMemo<Components>(() => {
    const openExternal = (url: string) =>
      void openUrl(url).catch((error) =>
        document.reportError(errorMessage(error)),
      );
    return {
      a: ({ href, title, children }) => {
        const target = markdownTarget(href ?? "", relative);
        if (!target) return <span>{children}</span>;
        return (
          <a
            href={href}
            title={title}
            onClick={(event) => {
              event.preventDefault();
              if (target.kind === "external") openExternal(target.value);
              else if (target.kind === "file") onOpenFile(root, target.value);
              else if (!target.value)
                content.current?.parentElement?.scrollTo({ top: 0 });
              else
                content.current
                  ?.querySelector<HTMLElement>(`#${CSS.escape(target.value)}`)
                  ?.scrollIntoView({ block: "start" });
            }}
          >
            {children}
          </a>
        );
      },
      img: ({ src, alt, title }) => {
        const target = markdownTarget(
          typeof src === "string" ? src : "",
          relative,
        );
        if (target?.kind === "file")
          return (
            <MarkdownImage
              root={root}
              relative={target.value}
              alt={alt ?? ""}
              title={title}
              assetPermit={assetPermit}
            />
          );
        if (target?.kind === "external")
          return (
            <a
              className="markdown-image-link"
              href={target.value}
              onClick={(event) => {
                event.preventDefault();
                openExternal(target.value);
              }}
            >
              Image: {alt || target.value}
            </a>
          );
        return <span>{alt}</span>;
      },
    };
  }, [document, onOpenFile, root, relative, assetPermit]);
  return (
    <section
      className="markdown-preview"
      aria-label={`Markdown preview for ${relative}`}
      tabIndex={0}
    >
      <article
        className="markdown-content"
        ref={content}
        aria-busy={deferred !== current}
      >
        <Markdown remarkPlugins={plugins} skipHtml components={components}>
          {text}
        </Markdown>
      </article>
    </section>
  );
});

function MarkdownImage({
  root,
  relative,
  alt,
  title,
  assetPermit,
}: {
  root: string;
  relative: string;
  alt: string;
  title?: string;
  assetPermit?: string | null;
}) {
  const [source, setSource] = useState("");
  const [error, setError] = useState("");
  useEffect(() => {
    let active = true;
    const reader = new FileReader();
    setSource("");
    setError("");
    if (assetPermit !== undefined) {
      void readAgentPreviewAsset(assetPermit, relative)
        .then((image) => {
          if (active)
            setSource(`data:${image.mimeType};base64,${image.dataBase64}`);
        })
        .catch((error) => {
          if (active) setError(errorMessage(error));
        });
      return () => {
        active = false;
      };
    }
    const types: Record<string, string> = {
      png: "image/png",
      jpg: "image/jpeg",
      jpeg: "image/jpeg",
      gif: "image/gif",
      webp: "image/webp",
      svg: "image/svg+xml",
      avif: "image/avif",
      ico: "image/x-icon",
    };
    const type = types[relative.split(".").pop()?.toLowerCase() ?? ""];
    if (!type) {
      setError("Unsupported image format");
      return;
    }
    reader.onload = () => {
      if (active) setSource(String(reader.result));
    };
    reader.onerror = () => {
      if (active) setError("Cannot read image");
    };
    void api<ArrayBuffer>("read_markdown_image", { root, relative })
      .then((bytes) => {
        if (active) reader.readAsDataURL(new Blob([bytes], { type }));
      })
      .catch((error) => {
        if (active) setError(errorMessage(error));
      });
    return () => {
      active = false;
      reader.abort();
    };
  }, [root, relative, assetPermit]);
  if (error)
    return (
      <span className="markdown-image-error" title={error}>
        {alt || relative} (image unavailable)
      </span>
    );
  return source ? (
    <img
      src={source}
      alt={alt}
      title={title}
      onError={() => setError("Cannot decode image")}
    />
  ) : (
    <span className="muted">{alt || "Loading image…"}</span>
  );
}
