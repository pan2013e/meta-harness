import { useEffect, useRef } from "react";
import { Uri, editor } from "monaco-editor/editor/editor.api.js";
import "monaco-editor/basic-languages/monaco.contribution.js";
import EditorWorker from "monaco-editor/editor/editor.worker.js?worker";
import { useAppStore } from "../../../store/useAppStore";

globalThis.MonacoEnvironment ??= { getWorker: () => new EditorWorker() };

export default function CodePreview({ value, language, filename }: {
  value: string;
  language: string;
  filename: string;
}) {
  const host = useRef<HTMLDivElement>(null);
  const theme = useAppStore((state) => state.theme);
  useEffect(() => {
    if (!host.current) return;
    const model = editor.createModel(value, language, Uri.from({ scheme: "inmemory", path: "/" + filename }));
    const instance = editor.create(host.current, {
      model,
      ariaLabel: filename,
      automaticLayout: true,
      readOnly: true,
      domReadOnly: true,
      minimap: { enabled: false },
      scrollBeyondLastLine: false,
      wordWrap: "on",
      fontSize: 12,
    });
    return () => {
      instance.dispose();
      model.dispose();
    };
  }, [filename, language, value]);
  useEffect(() => {
    editor.setTheme(theme === "daylight" ? "vs" : "vs-dark");
  }, [theme]);
  return <div ref={host} className="h-full w-full" />;
}
