import { useState, type ReactNode } from "react";
import { Check, ChevronRight, Copy, ExternalLink, FileCode2 } from "lucide-react";
import { sessionFileUrl } from "../../api/client";
import { useAppStore } from "../../store/useAppStore";

function extractWorktreeRelativePath(rawHref: string, projectRoot?: string | null): string | null {
  let clean = rawHref.trim();
  if (clean.startsWith("file:///")) clean = clean.slice(8);
  else if (clean.startsWith("file://")) clean = clean.slice(7);
  clean = clean.replace(/\\/g, "/");

  const wtMarker = ".meta-harness/worktrees/";
  const idx = clean.toLowerCase().indexOf(wtMarker);
  if (idx !== -1) {
    const rest = clean.slice(idx + wtMarker.length);
    const parts = rest.split("/");
    if (parts.length >= 3) {
      return parts.slice(2).join("/");
    }
  }

  if (projectRoot) {
    const normRoot = projectRoot.replace(/\\/g, "/").replace(/\/+$/, "");
    if (clean.toLowerCase().startsWith(`${normRoot.toLowerCase()}/`)) {
      return clean.slice(normRoot.length + 1);
    }
  }

  if (clean.startsWith("./")) return clean.slice(2);
  if (!/^[a-zA-Z]+:/.test(clean) && !clean.startsWith("/") && clean.length > 0) {
    return clean;
  }
  return null;
}

/**
 * Rich zero-dependency Markdown renderer supporting fenced code/diff blocks,
 * `<details><summary>` disclosure blocks, headings, lists, blockquotes,
 * pipe tables, local worktree preview links, inline code, emphasis, and @mentions.
 */
export function MarkdownBlock({
  content,
  sessionId: explicitSessionId,
}: {
  content: string;
  sessionId?: string | null;
}) {
  const storeSessionId = useAppStore((s) => s.activeSession?.id ?? null);
  const activeProjectRoot = useAppStore((s) => {
    const p = s.projects.find((item) => item.id === s.activeProjectId);
    return p?.root_path ?? null;
  });
  const sid = explicitSessionId !== undefined ? explicitSessionId : storeSessionId;

  const segments = splitFencesAndDetails(content);

  return (
    <div className="space-y-2.5 text-[13.5px] leading-relaxed text-ink break-words">
      {segments.map((seg, idx) =>
        seg.kind === "code" ? (
          <CodeFence key={idx} lang={seg.lang} code={seg.text} />
        ) : seg.kind === "details" ? (
          <details
            key={idx}
            className="rounded-xl border border-line bg-elevated/45 px-3 py-2 text-xs"
          >
            <summary className="cursor-pointer list-none flex items-center gap-1.5 font-medium text-ink-soft hover:text-ink [&::-webkit-details-marker]:hidden">
              <ChevronRight className="w-3.5 h-3.5 text-ink-muted shrink-0 details-chevron" />
              <span>{seg.summary}</span>
            </summary>
            <div className="mt-2 pt-2 border-t border-line text-ink-soft">
              <MarkdownProse text={seg.body} sessionId={sid} projectRoot={activeProjectRoot} />
            </div>
          </details>
        ) : (
          <MarkdownProse
            key={idx}
            text={seg.text}
            sessionId={sid}
            projectRoot={activeProjectRoot}
          />
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

type BlockSegment =
  | { kind: "text"; text: string }
  | { kind: "code"; lang: string; text: string }
  | { kind: "details"; summary: string; body: string };

function splitFencesAndDetails(text: string): BlockSegment[] {
  const out: BlockSegment[] = [];
  const fenceRegex = /```([a-zA-Z0-9_-]*)\r?\n([\s\S]*?)```/g;
  let last = 0;
  let m: RegExpExecArray | null;

  const pushProseWithDetails = (chunk: string) => {
    if (!chunk) return;
    const detailsRegex =
      /<details(?:\s+open)?>\s*<summary>([\s\S]*?)<\/summary>([\s\S]*?)<\/details>/gi;
    let dLast = 0;
    let dm: RegExpExecArray | null;
    while ((dm = detailsRegex.exec(chunk)) !== null) {
      if (dm.index > dLast) {
        out.push({ kind: "text", text: chunk.slice(dLast, dm.index) });
      }
      out.push({
        kind: "details",
        summary: dm[1].trim() || "Details",
        body: dm[2].trim(),
      });
      dLast = dm.index + dm[0].length;
    }
    if (dLast < chunk.length) {
      out.push({ kind: "text", text: chunk.slice(dLast) });
    }
  };

  while ((m = fenceRegex.exec(text)) !== null) {
    if (m.index > last) {
      pushProseWithDetails(text.slice(last, m.index));
    }
    out.push({ kind: "code", lang: m[1] || "", text: m[2].replace(/\r?\n$/, "") });
    last = m.index + m[0].length;
  }
  if (last < text.length) {
    pushProseWithDetails(text.slice(last));
  }
  if (out.length === 0) {
    out.push({ kind: "text", text });
  }
  return out;
}

function MarkdownProse({
  text,
  sessionId,
  projectRoot,
}: {
  text: string;
  sessionId: string | null;
  projectRoot: string | null;
}) {
  const lines = text.split(/\r?\n/);
  const nodes: ReactNode[] = [];
  let i = 0;

  while (i < lines.length) {
    const line = lines[i];

    if (!line.trim()) {
      i += 1;
      continue;
    }

    if (/^\s*(?:---|\*\*\*|___)\s*$/.test(line)) {
      nodes.push(<hr key={i} className="border-line my-2" />);
      i += 1;
      continue;
    }

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
          {renderInline(headingMatch[2], sessionId, projectRoot)}
        </div>
      );
      i += 1;
      continue;
    }

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
            <div key={qi}>{renderInline(q, sessionId, projectRoot)}</div>
          ))}
        </blockquote>
      );
      continue;
    }

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
                    {renderInline(h, sessionId, projectRoot)}
                  </th>
                ))}
              </tr>
            </thead>
            <tbody className="divide-y divide-line">
              {rows.map((r, ri) => (
                <tr key={ri} className="hover:bg-hover/40">
                  {headers.map((_, ci) => (
                    <td key={ci} className="px-2.5 py-1.5 text-ink-soft">
                      {renderInline(r[ci] ?? "", sessionId, projectRoot)}
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

    if (/^\s*[-*]\s+/.test(line)) {
      const items: string[] = [];
      while (i < lines.length && /^\s*[-*]\s+/.test(lines[i])) {
        items.push(lines[i].replace(/^\s*[-*]\s+/, ""));
        i += 1;
      }
      nodes.push(
        <ul key={i} className="list-disc pl-5 space-y-1 text-ink">
          {items.map((it, ii) => (
            <li key={ii}>{renderInline(it, sessionId, projectRoot)}</li>
          ))}
        </ul>
      );
      continue;
    }

    if (/^\s*\d+\.\s+/.test(line)) {
      const items: string[] = [];
      while (i < lines.length && /^\s*\d+\.\s+/.test(lines[i])) {
        items.push(lines[i].replace(/^\s*\d+\.\s+/, ""));
        i += 1;
      }
      nodes.push(
        <ol key={i} className="list-decimal pl-5 space-y-1 text-ink">
          {items.map((it, ii) => (
            <li key={ii}>{renderInline(it, sessionId, projectRoot)}</li>
          ))}
        </ol>
      );
      continue;
    }

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
        {renderInline(para.join("\n"), sessionId, projectRoot)}
      </div>
    );
  }

  return <div className="space-y-2">{nodes}</div>;
}

function renderInline(
  text: string,
  sessionId: string | null,
  projectRoot: string | null
): ReactNode[] {
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

    const richRegex =
      /(\*\*[^*\n]+\*\*|\[[^\]]+\]\([^)\s]+\)|\B@[a-zA-Z0-9_-]+(?:\[[a-zA-Z0-9._:-]+\])?)/g;
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
          const linkMatch = tok.match(/^\[([^\]]+)\]\(([^)\s]+)\)$/);
          if (linkMatch) {
            const [, label, rawHref] = linkMatch;
            if (/^https?:\/\//i.test(rawHref)) {
              return (
                <a
                  key={j}
                  href={rawHref}
                  target="_blank"
                  rel="noreferrer"
                  className="text-brand-ink underline decoration-brand/40 hover:decoration-brand"
                >
                  {label}
                </a>
              );
            }
            const rel = extractWorktreeRelativePath(rawHref, projectRoot);
            if (rel && sessionId) {
              return (
                <a
                  key={j}
                  href={sessionFileUrl(sessionId, rel)}
                  target="_blank"
                  rel="noreferrer"
                  title={rawHref}
                  className="inline-flex items-center gap-1 px-1.5 py-0.2 rounded-md bg-brand-soft text-brand-ink hover:underline font-medium text-[12.5px]"
                >
                  <FileCode2 className="w-3 h-3 shrink-0" />
                  <span>{label}</span>
                  <ExternalLink className="w-2.5 h-2.5 shrink-0 opacity-75" />
                </a>
              );
            }
            return (
              <span
                key={j}
                title={rawHref}
                className="inline-flex items-center gap-1 px-1.5 py-0.2 rounded-md bg-elevated border border-line text-ink-soft font-mono text-[11.5px]"
              >
                <FileCode2 className="w-3 h-3 shrink-0" />
                <span>{label}</span>
              </span>
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
