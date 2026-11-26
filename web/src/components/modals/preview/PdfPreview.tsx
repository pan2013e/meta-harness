import { useEffect, useRef, useState } from "react";
import { getDocument, GlobalWorkerOptions, type PDFDocumentProxy } from "pdfjs-dist";
import workerUrl from "pdfjs-dist/build/pdf.worker.min.mjs?url";

GlobalWorkerOptions.workerSrc = workerUrl;

export default function PdfPreview({ file }: { file: Blob }) {
  const canvas = useRef<HTMLCanvasElement>(null);
  const [document, setDocument] = useState<PDFDocumentProxy | null>(null);
  const [page, setPage] = useState(1);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let active = true;
    const url = URL.createObjectURL(file);
    const task = getDocument({ url, isOffscreenCanvasSupported: false });
    void task.promise.then((pdf) => {
      if (active) setDocument(pdf);
    }).catch(() => {
      if (active) setError("The PDF could not be opened.");
    });
    return () => {
      active = false;
      void task.destroy().catch(() => undefined);
      URL.revokeObjectURL(url);
    };
  }, [file]);

  useEffect(() => {
    const target = canvas.current;
    if (!document || !target) return;
    let active = true;
    let cancel: (() => void) | undefined;
    void document.getPage(page).then(async (pdfPage) => {
      if (!active) return;
      const original = pdfPage.getViewport({ scale: 1 });
      const viewport = pdfPage.getViewport({ scale: Math.min(2, 1400 / Math.max(original.width, original.height)) });
      target.width = Math.ceil(viewport.width);
      target.height = Math.ceil(viewport.height);
      const render = pdfPage.render({ canvas: target, viewport });
      cancel = () => render.cancel();
      await render.promise;
    }).catch(() => {
      if (active) setError("The PDF page could not be displayed.");
    });
    return () => {
      active = false;
      cancel?.();
    };
  }, [document, page]);

  if (error) return <p role="alert" className="p-5 text-sm text-ink-muted">{error}</p>;
  return (
    <div className="flex h-full flex-col">
      {document ? (
        <nav aria-label="PDF pages" className="flex shrink-0 items-center justify-center gap-3 border-b border-line bg-panel p-2 text-xs">
          <button type="button" onClick={() => setPage(page - 1)} disabled={page <= 1}
            className="rounded-md px-2 py-1 hover:bg-hover disabled:opacity-40">Previous</button>
          <span aria-live="polite">Page {page} of {document.numPages}</span>
          <button type="button" onClick={() => setPage(page + 1)} disabled={page >= document.numPages}
            className="rounded-md px-2 py-1 hover:bg-hover disabled:opacity-40">Next</button>
        </nav>
      ) : <p role="status" className="p-5 text-sm text-ink-muted">Opening PDF…</p>}
      <div className="min-h-0 flex-1 overflow-auto p-4">
        <canvas ref={canvas} aria-label={"PDF page " + page} className="mx-auto block h-auto max-w-full bg-white shadow-sm" />
      </div>
    </div>
  );
}
