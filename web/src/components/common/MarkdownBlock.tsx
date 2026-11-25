import { useState } from "react";
import { Check, Copy } from "lucide-react";

/** Lightweight zero-dependency Markdown renderer handling fenced code blocks, inline code, and @mentions cleanly. */
export function MarkdownBlock({ content }: { content: string }) {
  const segments = splitFencedBlocks(content);

  return (
    <div className="space-y-2 text-sm leading-relaxed text-zinc-200 break-words">
      {segments.map((seg, idx) =>
        seg.kind === "code" ? (
          <CodeFence key={idx} lang={seg.lang} code={seg.text} />
        ) : (
          <div key={idx} className="whitespace-pre-wrap">
            {renderInline(seg.text)}
          </div>
        )
      )}
    </div>
  );
}

function CodeFence({ lang, code }: { lang: string; code: string }) {
  const [copied, setCopied] = useState(false);

  const handleCopy = () => {
    void navigator.clipboard.writeText(code);
    setCopied(true);
    window.setTimeout(() => setCopied(false), 1500);
  };

  return (
    <div className="rounded-lg border border-zinc-800 bg-zinc-950 overflow-hidden my-1.5">
      <div className="flex items-center justify-between px-3 py-1 bg-zinc-900/80 border-b border-zinc-800 text-[11px] text-zinc-400">
        <span className="font-mono">{lang || "code"}</span>
        <button
          type="button"
          onClick={handleCopy}
          className="inline-flex items-center gap-1 hover:text-zinc-200 transition-colors"
        >
          {copied ? <Check className="w-3 h-3 text-emerald-400" /> : <Copy className="w-3 h-3" />}
          <span>{copied ? "Copied" : "Copy"}</span>
        </button>
      </div>
      <pre className="p-3 text-xs font-mono text-zinc-200 overflow-x-auto leading-5">
        <code>{code}</code>
      </pre>
    </div>
  );
}

function splitFencedBlocks(
  text: string
): Array<{ kind: "text"; text: string } | { kind: "code"; lang: string; text: string }> {
  const parts: Array<
    { kind: "text"; text: string } | { kind: "code"; lang: string; text: string }
  > = [];
  const regex = /```([a-zA-Z0-9_-]*)\n([\s\S]*?)```/g;
  let last = 0;
  let m: RegExpExecArray | null;
  while ((m = regex.exec(text)) !== null) {
    if (m.index > last) {
      parts.push({ kind: "text", text: text.slice(last, m.index) });
    }
    parts.push({ kind: "code", lang: m[1] || "", text: m[2].replace(/\n$/, "") });
    last = m.index + m[0].length;
  }
  if (last < text.length) {
    parts.push({ kind: "text", text: text.slice(last) });
  }
  if (parts.length === 0) {
    parts.push({ kind: "text", text });
  }
  return parts;
}

function renderInline(text: string) {
  // Split on inline code spans first so @mentions inside code aren't highlighted
  const chunks = text.split(/(`[^`]+`)/g);
  return chunks.map((chunk, i) => {
    if (chunk.startsWith("`") && chunk.endsWith("`") && chunk.length >= 2) {
      return (
        <code
          key={i}
          className="px-1.5 py-0.5 rounded bg-zinc-800/90 text-indigo-300 font-mono text-[12px]"
        >
          {chunk.slice(1, -1)}
        </code>
      );
    }

    const tokens = chunk.split(/(\B@[a-zA-Z0-9_-]+(?:\[[a-zA-Z0-9._:-]+\])?)/g);
    return (
      <span key={i}>
        {tokens.map((tok, j) =>
          tok.startsWith("@") ? (
            <span
              key={j}
              className="inline-block px-1 py-0.2 rounded bg-indigo-500/15 text-indigo-300 font-medium"
            >
              {tok}
            </span>
          ) : (
            <span key={j}>{tok}</span>
          )
        )}
      </span>
    );
  });
}
