import { Suspense, lazy, useEffect, useRef, useState } from "react";
import { Download, X } from "lucide-react";
import { sessionFileUrl, sessionPreviewUrl } from "../../api/client";

const CodePreview = lazy(() => import("./preview/CodePreview"));
const PdfPreview = lazy(() => import("./preview/PdfPreview"));

const CODE_LANGUAGES: Readonly<Record<string, string>> = {
  bat: "bat", c: "cpp", cc: "cpp", cpp: "cpp", cs: "csharp", css: "css",
  go: "go", h: "cpp", hpp: "cpp", ini: "ini",
  java: "java", js: "javascript", json: "json", jsx: "javascript",
  kt: "kotlin", md: "markdown", mjs: "javascript", php: "php", ps1: "powershell",
  py: "python", rb: "ruby", rs: "rust", sh: "shell", sql: "sql", swift: "swift",
  toml: "ini", ts: "typescript", tsx: "typescript", txt: "plaintext",
  xml: "xml", yaml: "yaml", yml: "yaml",
};

type Preview =
  | { type: "loading" }
  | { type: "html"; src: string }
  | { type: "image"; url: string }
  | { type: "pdf"; file: Blob }
  | { type: "code"; value: string; language: string }
  | { type: "message"; text: string };

export function FilePreviewModal({ sessionId, path, onClose }: {
  sessionId: string;
  path: string;
  onClose: () => void;
}) {
  const dialog = useRef<HTMLDialogElement>(null);
  const [preview, setPreview] = useState<Preview>({ type: "loading" });
  const url = sessionFileUrl(sessionId, path);
  const filename = path.split(/[\\/]/).pop() || path;

  useEffect(() => {
    if (!dialog.current?.open) dialog.current?.showModal();
  }, []);

  useEffect(() => {
    const abort = new AbortController();
    let imageUrl: string | undefined;
    const load = async () => {
      try {
        const previewUrl = url + "?preview=true";
        const htmlFile = /\.html?$/i.test(filename);
        const response = await fetch(previewUrl, {
          method: htmlFile ? "HEAD" : "GET",
          signal: abort.signal, credentials: "same-origin",
        });
        if (!response.ok) {
          const message = response.status === 413 ? "This file is too large to preview." : "The file could not be opened.";
          throw new Error(message);
        }
        if (abort.signal.aborted) return;
        const mime = response.headers.get("content-type")?.split(";")[0].trim().toLowerCase() ?? "";
        if (mime === "text/html") {
          void response.body?.cancel().catch(() => undefined);
          const grant = response.headers.get("x-meta-harness-preview");
          if (!grant) throw new Error("The server could not start the webpage preview.");
          setPreview({ type: "html", src: sessionPreviewUrl(sessionId, grant, path) });
          return;
        }
        const file = await response.blob();
        if (abort.signal.aborted) return;
        if (mime.startsWith("image/")) {
          imageUrl = URL.createObjectURL(file);
          setPreview({ type: "image", url: imageUrl });
          return;
        }
        if (mime === "application/pdf") {
          setPreview({ type: "pdf", file });
          return;
        }
        if (file.size > 1024 * 1024) throw new Error("This file is too large to preview as text.");
        const value = new TextDecoder("utf-8", { fatal: true }).decode(await file.arrayBuffer());
        if (value.includes("\0")) throw new Error("Preview is unavailable for this file type.");
        const extension = filename.split(".").pop()?.toLowerCase() ?? "";
        if (!abort.signal.aborted) setPreview({ type: "code", value, language: CODE_LANGUAGES[extension] || "plaintext" });
      } catch (error) {
        if (!abort.signal.aborted) {
          setPreview({
            type: "message",
            text: error instanceof TypeError
              ? "Preview is unavailable for this file type."
              : error instanceof Error ? error.message : "The file could not be opened.",
          });
        }
      }
    };
    void load();
    return () => {
      abort.abort();
      if (imageUrl) URL.revokeObjectURL(imageUrl);
    };
  }, [filename, path, sessionId, url]);

  const loading = <p role="status" className="p-5 text-sm text-ink-muted">Loading preview…</p>;
  return (
    <dialog ref={dialog} onClose={onClose} aria-labelledby="file-preview-title"
      className="m-auto h-[min(84dvh,760px)] w-[calc(100%-2rem)] max-w-5xl overflow-hidden rounded-xl border border-line-strong bg-panel p-0 text-ink shadow-2xl">
      <div className="flex h-full flex-col">
        <header className="flex shrink-0 items-center gap-3 border-b border-line px-4 py-2.5">
          <h2 id="file-preview-title" className="min-w-0 flex-1 truncate text-sm font-medium" title={path}>{filename}</h2>
          <a href={url} download={filename} className="inline-flex items-center gap-1.5 rounded-md px-2 py-1 text-xs text-ink-soft hover:bg-hover">
            <Download className="h-3.5 w-3.5" /> Download
          </a>
          <button type="button" aria-label="Close file preview" onClick={() => dialog.current?.close()}
            className="rounded-md p-1.5 text-ink-muted hover:bg-hover hover:text-ink">
            <X className="h-4 w-4" />
          </button>
        </header>
        <div className="min-h-0 flex-1 overflow-auto bg-inset">
          <Suspense fallback={loading}>
            {preview.type === "loading" && loading}
            {preview.type === "message" && <p className="p-5 text-sm text-ink-muted">{preview.text}</p>}
            {preview.type === "html" && (
              <iframe src={preview.src} title={filename + " webpage"}
                sandbox="allow-scripts" referrerPolicy="no-referrer"
                className="block h-full w-full border-0 bg-white" />
            )}
            {preview.type === "image" && (
              <div className="flex h-full items-center justify-center p-4">
                <img src={preview.url} alt={filename} className="max-h-full max-w-full object-contain"
                  onError={() => setPreview({ type: "message", text: "The image could not be displayed." })} />
              </div>
            )}
            {preview.type === "code" && <CodePreview value={preview.value} language={preview.language} filename={filename} />}
            {preview.type === "pdf" && <PdfPreview file={preview.file} />}
          </Suspense>
        </div>
      </div>
    </dialog>
  );
}
