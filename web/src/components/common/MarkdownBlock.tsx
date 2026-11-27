import { Children, useMemo, useState, type ReactNode } from "react";
import { Check, ChevronRight, Copy, ExternalLink, FileCode2 } from "lucide-react";
import Markdown, { type Components } from "react-markdown";
import remarkGfm from "remark-gfm";
import { SessionFileLink } from "./SessionFileLink";
import { useAppStore } from "../../store/useAppStore";

function extractWorktreeRelativePath(rawHref: string, projectRoot?: string | null): string | null {
  let clean = rawHref.trim().split(/[?#]/, 1)[0];
  if (/^file:\/\//i.test(clean)) {
    try {
      const file = new URL(clean);
      if (file.hostname && file.hostname !== "localhost") return null;
      clean = file.pathname.replace(/^\/(?=[a-z]:\/)/i, "");
    } catch {
      return null;
    }
  }
  try {
    clean = decodeURIComponent(clean);
  } catch {
    return null;
  }
  clean = clean.replace(/\\/g, "/");
  if (clean.startsWith("//") || (/^[a-z][a-z0-9+.-]*:/i.test(clean) && !/^[a-z]:\//i.test(clean))) return null;
  clean = clean.replace(/:\d+(?::\d+)?$/, "");

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
  const skillSlugsKey = useAppStore((s) =>
    s.skills.map((sk) => sk.id.toLowerCase()).join(",")
  );
  const sid = explicitSessionId !== undefined ? explicitSessionId : storeSessionId;
  const components = useMemo(() => markdownComponents(sid, activeProjectRoot, new Set(skillSlugsKey.split(","))), [sid, activeProjectRoot, skillSlugsKey]);
  const sections = useMemo(() => splitDetails(content), [content]);
  const renderMarkdown = (source: string) => (
    <Markdown remarkPlugins={markdownPlugins} components={components} urlTransform={linkDestination}>{source}</Markdown>
  );

  return (
    <div className="space-y-2.5 text-[13.5px] leading-relaxed text-ink break-words">
      {sections.map((section, idx) =>
        section.kind === "details" ? (
          <details
            key={idx}
            className="rounded-xl border border-line bg-elevated/45 px-3 py-2 text-xs"
          >
            <summary className="cursor-pointer list-none flex items-center gap-1.5 font-medium text-ink-soft hover:text-ink [&::-webkit-details-marker]:hidden">
              <ChevronRight className="w-3.5 h-3.5 text-ink-muted shrink-0 details-chevron" />
              <span>{section.summary}</span>
            </summary>
            <div className="mt-2 space-y-2 border-t border-line pt-2 text-ink-soft">
              {renderMarkdown(section.body)}
            </div>
          </details>
        ) : (
          <div key={idx} className="space-y-2">{renderMarkdown(section.text)}</div>
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

type MarkdownSection =
  | { kind: "text"; text: string }
  | { kind: "details"; summary: string; body: string };

const markdownPlugins = [remarkGfm];

// Preserve the destination for the app's link renderer, which validates it
// before producing a navigation target, including Windows worktree paths.
const linkDestination = (url: string) => url;

function splitDetails(text: string): MarkdownSection[] {
  if (!/<details/i.test(text)) return [{ kind: "text", text }];
  const codeRanges: Array<{ start: number; end: number }> = [];
  let fence: { marker: string; start: number } | undefined;
  for (const match of text.matchAll(/^[ \t]*(\x60{3,}|~{3,})([^\r\n]*)/gm)) {
    const [, marker, rest] = match;
    if (!fence) {
      fence = { marker, start: match.index };
    } else if (marker.startsWith(fence.marker) && !rest.trim()) {
      codeRanges.push({ start: fence.start, end: match.index + match[0].length });
      fence = undefined;
    }
  }
  if (fence) codeRanges.push({ start: fence.start, end: text.length });

  const sections: MarkdownSection[] = [];
  let start = 0;
  let rangeIndex = 0;
  const details = /^[ \t]*<details(?:[ \t]+open)?>\s*<summary>([\s\S]*?)<\/summary>([\s\S]*?)<\/details>/gim;
  for (const match of text.matchAll(details)) {
    while (codeRanges[rangeIndex]?.end <= match.index) rangeIndex++;
    const code = codeRanges[rangeIndex];
    if (code && code.start <= match.index) continue;
    if (start < match.index) sections.push({ kind: "text", text: text.slice(start, match.index) });
    sections.push({ kind: "details", summary: match[1].trim() || "Details", body: match[2].trim() });
    start = match.index + match[0].length;
  }
  if (start < text.length || !sections.length) sections.push({ kind: "text", text: text.slice(start) });
  return sections;
}

function markdownComponents(
  sessionId: string | null,
  projectRoot: string | null,
  skillSlugs: ReadonlySet<string>
): Components {
  const decorate = (children: ReactNode) =>
    Children.map(children, (child) => typeof child === "string" ? decorateText(child, skillSlugs) : child);

  const renderLink = (href: string | undefined, children: ReactNode) => {
    if (!href) return children;
    if (/^https?:\/\//i.test(href)) {
      return (
        <a href={href} target="_blank" rel="noreferrer" className="text-brand-ink underline decoration-brand/40 hover:decoration-brand">
          {children}
        </a>
      );
    }
    const path = extractWorktreeRelativePath(href, projectRoot);
    if (path && sessionId) {
      return (
        <SessionFileLink
          sessionId={sessionId}
          path={path}
          title={href}
          className="inline-flex items-center gap-1 rounded-md bg-brand-soft px-1.5 py-0.2 text-[12.5px] font-medium text-brand-ink hover:underline"
        >
          <FileCode2 className="h-3 w-3 shrink-0" />
          <span>{children}</span>
          <ExternalLink className="h-2.5 w-2.5 shrink-0 opacity-75" />
        </SessionFileLink>
      );
    }
    return <span title={href}>{children}</span>;
  };

  return {
    p: ({ children }) => <p className="whitespace-pre-wrap">{decorate(children)}</p>,
    h1: ({ children }) => <h1 className="mt-2 text-base font-bold">{decorate(children)}</h1>,
    h2: ({ children }) => <h2 className="mt-1.5 text-[14.5px] font-semibold">{decorate(children)}</h2>,
    h3: ({ children }) => <h3 className="mt-1 text-[13.5px] font-semibold text-ink-soft">{decorate(children)}</h3>,
    h4: ({ children }) => <h4 className="mt-1 font-semibold text-ink-soft">{decorate(children)}</h4>,
    h5: ({ children }) => <h5 className="mt-1 text-[12.5px] font-semibold text-ink-soft">{decorate(children)}</h5>,
    h6: ({ children }) => <h6 className="mt-1 text-[12px] font-semibold text-ink-soft">{decorate(children)}</h6>,
    strong: ({ children }) => <strong className="font-semibold">{decorate(children)}</strong>,
    em: ({ children }) => <em>{decorate(children)}</em>,
    del: ({ children }) => <del>{decorate(children)}</del>,
    blockquote: ({ children }) => <blockquote className="space-y-1 border-l-2 border-brand py-0.5 pl-3 italic text-ink-soft">{children}</blockquote>,
    ol: ({ start, children }) => <ol start={start} className="list-decimal space-y-1 pl-5">{children}</ol>,
    ul: ({ children }) => <ul className="list-disc space-y-1 pl-5">{children}</ul>,
    li: ({ children }) => <li className="space-y-1 [&>ul]:list-[circle]">{decorate(children)}</li>,
    hr: () => <hr className="my-2 border-line" />,
    a: ({ href, children }) => renderLink(href, children),
    img: ({ src, alt }) => renderLink(src, alt || "Image"),
    code: ({ children }) => (
      <code className="rounded-md border border-line bg-elevated px-1.5 py-0.5 font-mono text-[12px] text-brand-ink">{children}</code>
    ),
    pre: ({ node, children }) => {
      const code = node?.children[0];
      if (code?.type !== "element" || code.tagName !== "code") return <pre>{children}</pre>;
      const classes = code.properties.className;
      const language = Array.isArray(classes) ? classes.find((name) => typeof name === "string" && name.startsWith("language-")) : undefined;
      const text = code.children.map((child) => child.type === "text" ? child.value : "").join("").replace(/\n$/, "");
      return <CodeFence lang={typeof language === "string" ? language.slice("language-".length) : ""} code={text} />;
    },
    table: ({ children }) => (
      <div className="my-1 overflow-x-auto rounded-lg border border-line">
        <table className="w-full border-collapse text-left text-xs">{children}</table>
      </div>
    ),
    thead: ({ children }) => <thead className="border-b border-line bg-elevated">{children}</thead>,
    tbody: ({ children }) => <tbody className="divide-y divide-line">{children}</tbody>,
    th: ({ children, style }) => <th style={style} className="px-2.5 py-1.5 font-semibold">{decorate(children)}</th>,
    td: ({ children, style }) => <td style={style} className="px-2.5 py-1.5 text-ink-soft">{decorate(children)}</td>,
  };
}

function decorateText(text: string, skillSlugs: ReadonlySet<string>): ReactNode[] {
  return text.split(/(\B@[a-zA-Z0-9_-]+(?:\[[a-zA-Z0-9._:-]+\])?|\B\/[a-zA-Z0-9_-]+(?=$|[\s.,!?;:]))/g)
    .map((part, index) => {
      if (index % 2 === 0) return part;
      if (part.startsWith("@")) {
        return <span key={index} className="inline-block rounded-md bg-brand-soft px-1.5 py-0.2 font-mono text-[12.5px] font-medium text-brand-ink">{part}</span>;
      }
      if (skillSlugs.has(part.slice(1).toLowerCase())) {
        return <span key={index} className="inline-block rounded-md border border-skill/35 bg-skill-soft px-1.5 py-0.2 font-mono text-[12.5px] font-medium text-skill">{part}</span>;
      }
      return part;
    });
}
