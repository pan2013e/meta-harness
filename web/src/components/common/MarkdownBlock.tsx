import { useState, type ReactNode } from "react";
import { Check, Copy } from "lucide-react";

/**
 * Rich zero-dependency Markdown renderer supporting fenced code/diff blocks,
 * headings, lists, blockquotes, pipe tables, inline code, links, emphasis, and @mentions.
 */
export function MarkdownBlock({ content }: { content: string }) {
  const segments = splitFencedBlocks(content);

  return (
    <div className="space-y-2.5 text-[13.5px] leading-relaxed text-ink break-words">
      {segments.map((seg, idx) =>
        seg.kind === "code" ? (
          <CodeFence key={idx} lang={seg.lang} code={seg.text} />
        ) : (
          <MarkdownProse key={idx} text={seg.text} />
        )
      )}
    </div>
  );
}

function CodeFence({ lang, code }: { lang: string; code: string }) {
  const [copied, setCopied] = useState(false);
  const isDiff = lang.toLowerCase() === "diff" || lang.toLowerCase() === "patch";
  const lines = code.split("\n");

  const handleCopy = () => {
    void navigator.clipboard.writeText(code);
    setCopied(true);
    window.setTimeout(() => setCopied(false), 1500);
  };

  return (
    <div className="rounded-xl border border-line bg-inset overflow-hidden my-1.5 shadow-sm">
      <div className="flex items-center justify-between px-3 py-1.5 bg-elevated border-b border-line text-[11px] text-ink-muted">
        <div className="flex items-center gap-2 font-mono">
          <span className="text-ink-soft font-medium">{lang || "code"}</span>
          <span>· {lines.length} lines</span>
        </div>
        <button
          type="button"
          onClick={handleCopy}
          className="inline-flex items-center gap-1 hover:text-ink transition-colors"
        >
          {copied ? (
            <Check className="w-3 h-3 text-pos" />
          ) : (
            <Copy className="w-3 h-3" />
          )}
          <span>{copied ? "Copied" : "Copy"}</span>
        </button>
      </div>
      <pre className="p-3 text-xs font-mono text-ink overflow-x-auto leading-5">
        {isDiff ? (
          <code>
            {lines.map((ln, idx) => {
              const cls = ln.startsWith("@@")
                ? "bg-brand-soft text-brand-ink"
                : ln.startsWith("+") && !ln.startsWith("+++")
                  ? "bg-pos-soft text-pos"
                  : ln.startsWith("-") && !ln.startsWith("---")
                    ? "bg-neg-soft text-neg"
                    : "";
              return (
                <div key={idx} className={`px-1 rounded ${cls}`}>
                  {ln || " "}
                </div>
              );
            })}
          </code>
        ) : (
          <code>{code}</code>
        )}
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
  const regex = /```([a-zA-Z0-9_-]*)\r?\n([\s\S]*?)```/g;
  let last = 0;
  let m: RegExpExecArray | null;
  while ((m = regex.exec(text)) !== null) {
    if (m.index > last) {
      parts.push({ kind: "text", text: text.slice(last, m.index) });
    }
    parts.push({ kind: "code", lang: m[1] || "", text: m[2].replace(/\r?\n$/, "") });
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

function MarkdownProse({ text }: { text: string }) {
  const lines = text.split(/\r?\n/);
  const nodes: ReactNode[] = [];
  let i = 0;

  while (i < lines.length) {
    const line = lines[i];

    if (!line.trim()) {
      i += 1;
      continue;
    }

    // Horizontal rule
    if (/^\s*(?:---|\*\*\*|___)\s*$/.test(line)) {
      nodes.push(<hr key={i} className="border-line my-2" />);
      i += 1;
      continue;
    }

    // Headings (#, ##, ###)
    const headingMatch = line.match(/^(#{1,3})\s+(.+)$/);
    if (headingMatch) {
      const level = headingMatch[1].length;
      const cls =
        level === 1
          ? "text-base font-bold text-ink mt-2"
          : level === 2
            ? "text-[14.5px] font-semibold text-ink mt-1.5"
            : "text-[13.5px] font-semibold text-ink-soft mt-1";
      nodes.push(
        <div key={i} className={cls}>
          {renderInline(headingMatch[2])}
        </div>
      );
      i += 1;
      continue;
    }

    // Blockquote
    if (line.trimStart().startsWith("> ")) {
      const quoteLines: string[] = [];
      while (i < lines.length && lines[i].trimStart().startsWith("> ")) {
        quoteLines.push(lines[i].replace(/^\s*>\s?/, ""));
        i += 1;
      }
      nodes.push(
        <blockquote
          key={i}
          className="border-l-2 border-brand pl-3 py-0.5 text-ink-soft italic space-y-1"
        >
          {quoteLines.map((q, qi) => (
            <div key={qi}>{renderInline(q)}</div>
          ))}
        </blockquote>
      );
      continue;
    }

    // Pipe Table
    if (
      line.includes("|") &&
      i + 1 < lines.length &&
      /^\s*\|?[\s:-]+\|[\s|:-]*$/.test(lines[i + 1])
    ) {
      const parseCells = (row: string) =>
        row
          .trim()
          .replace(/^\|/, "")
          .replace(/\|$/, "")
          .split("|")
          .map((c) => c.trim());
      const headers = parseCells(line);
      i += 2;
      const rows: string[][] = [];
      while (i < lines.length && lines[i].includes("|") && lines[i].trim()) {
        rows.push(parseCells(lines[i]));
        i += 1;
      }
      nodes.push(
        <div key={i} className="overflow-x-auto rounded-lg border border-line my-1">
          <table className="w-full text-left border-collapse text-xs">
            <thead className="bg-elevated border-b border-line text-ink">
              <tr>
                {headers.map((h, hi) => (
                  <th key={hi} className="px-2.5 py-1.5 font-semibold">
                    {renderInline(h)}
                  </th>
                ))}
              </tr>
            </thead>
            <tbody className="divide-y divide-line">
              {rows.map((r, ri) => (
                <tr key={ri} className="hover:bg-hover/40">
                  {headers.map((_, ci) => (
                    <td key={ci} className="px-2.5 py-1.5 text-ink-soft">
                      {renderInline(r[ci] ?? "")}
                    </td>
                  ))}
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      );
      continue;
    }

    // Unordered list
    if (/^\s*[-*]\s+/.test(line)) {
      const items: string[] = [];
      while (i < lines.length && /^\s*[-*]\s+/.test(lines[i])) {
        items.push(lines[i].replace(/^\s*[-*]\s+/, ""));
        i += 1;
      }
      nodes.push(
        <ul key={i} className="list-disc pl-5 space-y-1 text-ink">
          {items.map((it, ii) => (
            <li key={ii}>{renderInline(it)}</li>
          ))}
        </ul>
      );
      continue;
    }

    // Ordered list
    if (/^\s*\d+\.\s+/.test(line)) {
      const items: string[] = [];
      while (i < lines.length && /^\s*\d+\.\s+/.test(lines[i])) {
        items.push(lines[i].replace(/^\s*\d+\.\s+/, ""));
        i += 1;
      }
      nodes.push(
        <ol key={i} className="list-decimal pl-5 space-y-1 text-ink">
          {items.map((it, ii) => (
            <li key={ii}>{renderInline(it)}</li>
          ))}
        </ol>
      );
      continue;
    }

    // Paragraph (gather contiguous normal lines)
    const para: string[] = [];
    while (
      i < lines.length &&
      lines[i].trim() &&
      !/^(#{1,3})\s+/.test(lines[i]) &&
      !/^\s*[-*]\s+/.test(lines[i]) &&
      !/^\s*\d+\.\s+/.test(lines[i]) &&
      !lines[i].trimStart().startsWith("> ") &&
      !/^\s*(?:---|\*\*\*|___)\s*$/.test(lines[i])
    ) {
      para.push(lines[i]);
      i += 1;
    }
    nodes.push(
      <div key={i} className="whitespace-pre-wrap">
        {renderInline(para.join("\n"))}
      </div>
    );
  }

  return <div className="space-y-2">{nodes}</div>;
}

function renderInline(text: string): ReactNode[] {
  // 1. Split inline code spans first so @mentions or ** inside code aren't parsed
  const chunks = text.split(/(`[^`]+`)/g);
  return chunks.map((chunk, i) => {
    if (chunk.startsWith("`") && chunk.endsWith("`") && chunk.length >= 2) {
      return (
        <code
          key={i}
          className="px-1.5 py-0.5 rounded-md bg-elevated border border-line text-brand-ink font-mono text-[12px]"
        >
          {chunk.slice(1, -1)}
        </code>
      );
    }

    // 2. Parse bold, links, and @mentions outside code spans
    const richRegex =
      /(\*\*[^*\n]+\*\*|\[[^\]]+\]\((?:https?:\/\/|file:\/\/)[^)\s]+\)|\B@[a-zA-Z0-9_-]+(?:\[[a-zA-Z0-9._:-]+\])?)/g;
    const parts = chunk.split(richRegex);
    return (
      <span key={i}>
        {parts.map((tok, j) => {
          if (!tok) return null;
          if (tok.startsWith("**") && tok.endsWith("**") && tok.length > 4) {
            return (
              <strong key={j} className="font-semibold text-ink">
                {tok.slice(2, -2)}
              </strong>
            );
          }
          const linkMatch = tok.match(/^\[([^\]]+)\]\(((?:https?:\/\/|file:\/\/)[^)\s]+)\)$/);
          if (linkMatch) {
            return (
              <a
                key={j}
                href={linkMatch[2]}
                target="_blank"
                rel="noreferrer"
                className="text-brand-ink underline decoration-brand/40 hover:decoration-brand"
              >
                {linkMatch[1]}
              </a>
            );
          }
          if (tok.startsWith("@")) {
            return (
              <span
                key={j}
                className="inline-block px-1.5 py-0.2 rounded-md bg-brand-soft text-brand-ink font-medium font-mono text-[12.5px]"
              >
                {tok}
              </span>
            );
          }
          return <span key={j}>{tok}</span>;
        })}
      </span>
    );
  });
}
