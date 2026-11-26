import {
  forwardRef,
  useCallback,
  useEffect,
  useImperativeHandle,
  useRef,
} from "react";

export type SlashCommandKind = "workflow" | "action";

export const DEFAULT_SLASH_COMMANDS: ReadonlyMap<string, SlashCommandKind> = new Map([
  ["plan", "workflow"],
  ["review", "workflow"],
  ["test", "workflow"],
  ["fix", "workflow"],
  ["compact", "action"],
  ["diff", "action"],
  ["shell", "action"],
  ["memory", "action"],
  ["new", "action"],
  ["archive", "action"],
]);

export interface MentionEditorHandle {
  focus: () => void;
  insertMention: (handle: string) => void;
  insertSlash: (command: string) => void;
}

interface MentionEditorProps {
  value: string;
  validHandles: Set<string>;
  slashCommands?: ReadonlyMap<string, SlashCommandKind>;
  placeholder?: string;
  disabled?: boolean;
  onChange: (nextValue: string) => void;
  onMentionQuery: (query: string | null) => void;
  onSlashQuery?: (query: string | null) => void;
  onKeyDown?: (event: React.KeyboardEvent<HTMLDivElement>) => void;
}

/**
 * Serializes a contentEditable subtree containing atomic `data-mention` and
 * `data-slash` pills back into plain text (preserving `@handle` and `/command`).
 */
export function serializeMentionDom(root: HTMLElement): string {
  const walk = (node: Node): string => {
    if (node.nodeType === 3) {
      return (node.textContent ?? "").replace(/\u200B/g, "");
    }
    if (node.nodeType !== 1) return "";
    const el = node as HTMLElement;
    const mention = el.dataset?.mention;
    if (mention) {
      return `@${mention}`;
    }
    const slash = el.dataset?.slash;
    if (slash) {
      return `/${slash}`;
    }
    if (el.tagName === "BR") {
      return "\n";
    }
    let inner = "";
    const children = el.childNodes;
    for (let i = 0; i < children.length; i++) {
      inner += walk(children[i]);
    }
    if (el.tagName === "DIV" || el.tagName === "P") {
      if (el.previousSibling && !inner.startsWith("\n")) {
        return `\n${inner}`;
      }
    }
    return inner;
  };

  let raw = "";
  for (let i = 0; i < root.childNodes.length; i++) {
    raw += walk(root.childNodes[i]);
  }
  return raw.replace(/\r/g, "").replace(/\n$/, "");
}

/**
 * Detects an open `@prefix` mention token immediately preceding the caret,
 * returning `null` if none is active.
 */
export function findActiveMentionTrigger(textBeforeCaret: string): string | null {
  const clean = textBeforeCaret.replace(/\u200B/g, "");
  const m = clean.match(/(?:^|\s)@([a-zA-Z0-9_-]*)$/);
  return m ? m[1].toLowerCase() : null;
}

/**
 * Detects an open `/prefix` slash-command token immediately preceding the caret,
 * returning `null` if none is active.
 */
export function findActiveSlashTrigger(textBeforeCaret: string): string | null {
  const clean = textBeforeCaret.replace(/\u200B/g, "");
  const m = clean.match(/(?:^|\s)\/([a-zA-Z0-9_-]*)$/);
  return m ? m[1].toLowerCase() : null;
}

function makePillElement(handle: string, doc: Document): HTMLSpanElement {
  const span = doc.createElement("span");
  span.contentEditable = "false";
  span.dataset.mention = handle;
  span.className =
    "inline-flex items-center gap-0.5 align-baseline mx-0.5 px-1.5 py-0.2 rounded-md bg-brand-soft border border-brand/35 text-brand-ink font-mono text-[12.5px] font-semibold select-none";
  span.appendChild(doc.createTextNode(`@${handle}`));

  const btn = doc.createElement("button");
  btn.type = "button";
  btn.tabIndex = -1;
  btn.setAttribute("aria-label", `Remove @${handle}`);
  btn.dataset.removeMention = handle;
  btn.className =
    "ml-0.5 text-brand-ink/70 hover:text-ink leading-none cursor-pointer focus:outline-none";
  btn.textContent = "×";
  span.appendChild(btn);

  return span;
}

function makeSlashPillElement(
  command: string,
  kind: SlashCommandKind,
  doc: Document
): HTMLSpanElement {
  const span = doc.createElement("span");
  span.contentEditable = "false";
  span.dataset.slash = command;
  span.dataset.slashKind = kind;
  span.className =
    kind === "workflow"
      ? "inline-flex items-center gap-0.5 align-baseline mx-0.5 px-1.5 py-0.2 rounded-md bg-pos-soft border border-pos/35 text-pos font-mono text-[12.5px] font-semibold select-none"
      : "inline-flex items-center gap-0.5 align-baseline mx-0.5 px-1.5 py-0.2 rounded-md bg-warn-soft border border-warn/35 text-warn font-mono text-[12.5px] font-semibold select-none";
  span.appendChild(doc.createTextNode(`/${command}`));

  const btn = doc.createElement("button");
  btn.type = "button";
  btn.tabIndex = -1;
  btn.setAttribute("aria-label", `Remove /${command}`);
  btn.dataset.removeSlash = command;
  btn.className =
    kind === "workflow"
      ? "ml-0.5 text-pos/75 hover:text-ink leading-none cursor-pointer focus:outline-none"
      : "ml-0.5 text-warn/75 hover:text-ink leading-none cursor-pointer focus:outline-none";
  btn.textContent = "×";
  span.appendChild(btn);

  return span;
}

function buildFragmentFromValue(
  text: string,
  validHandles: Set<string>,
  slashCommands: ReadonlyMap<string, SlashCommandKind>,
  doc: Document
): DocumentFragment {
  const frag = doc.createDocumentFragment();
  if (!text) return frag;

  const lines = text.split("\n");
  const lowerToCanonical = new Map<string, string>();
  for (const h of validHandles) {
    lowerToCanonical.set(h.toLowerCase(), h);
  }

  lines.forEach((line, lineIdx) => {
    if (lineIdx > 0) {
      frag.appendChild(doc.createElement("br"));
    }
    const regex = /(^|\s)([@/])([a-zA-Z0-9_-]+)(?=$|[\s.,!?;:])/g;
    let last = 0;
    let match: RegExpExecArray | null;
    while ((match = regex.exec(line)) !== null) {
      const prefix = match[1];
      const sigil = match[2];
      const rawToken = match[3];
      let pill: HTMLSpanElement | null = null;
      if (sigil === "@") {
        const canonical = lowerToCanonical.get(rawToken.toLowerCase());
        if (canonical) pill = makePillElement(canonical, doc);
      } else if (sigil === "/") {
        const lowerCmd = rawToken.toLowerCase();
        const kind = slashCommands.get(lowerCmd);
        if (kind) pill = makeSlashPillElement(lowerCmd, kind, doc);
      }
      if (!pill) continue;
      const tokenStart = match.index + prefix.length;
      if (tokenStart > last) {
        frag.appendChild(doc.createTextNode(line.slice(last, tokenStart)));
      }
      frag.appendChild(pill);
      last = match.index + match[0].length;
    }
    if (last < line.length) {
      frag.appendChild(doc.createTextNode(line.slice(last)));
    }
  });

  return frag;
}

export const MentionEditor = forwardRef<MentionEditorHandle, MentionEditorProps>(
  function MentionEditor(
    {
      value,
      validHandles,
      slashCommands = DEFAULT_SLASH_COMMANDS,
      placeholder,
      disabled = false,
      onChange,
      onMentionQuery,
      onSlashQuery,
      onKeyDown,
    },
    ref
  ) {
    const editorRef = useRef<HTMLDivElement | null>(null);
    const lastEmittedRef = useRef<string>(value);

    const placeCaretAtEnd = useCallback((el: HTMLElement) => {
      const sel = window.getSelection();
      if (!sel) return;
      const range = el.ownerDocument.createRange();
      range.selectNodeContents(el);
      range.collapse(false);
      sel.removeAllRanges();
      sel.addRange(range);
    }, []);

    // Sync external value changes (send clear, starter prompt insert, error restore) without disrupting typing.
    useEffect(() => {
      const el = editorRef.current;
      if (!el) return;
      if (value === lastEmittedRef.current && serializeMentionDom(el) === value) {
        return;
      }
      lastEmittedRef.current = value;
      el.innerHTML = "";
      if (value) {
        el.appendChild(
          buildFragmentFromValue(value, validHandles, slashCommands, el.ownerDocument)
        );
        if (document.activeElement === el) {
          placeCaretAtEnd(el);
        }
      }
    }, [value, validHandles, slashCommands, placeCaretAtEnd]);

    const emitCurrent = useCallback(() => {
      const el = editorRef.current;
      if (!el) return;
      const serialized = serializeMentionDom(el);
      lastEmittedRef.current = serialized;
      onChange(serialized);

      const sel = window.getSelection();
      if (!sel || sel.rangeCount === 0 || !sel.isCollapsed) {
        onMentionQuery(null);
        onSlashQuery?.(null);
        return;
      }
      const range = sel.getRangeAt(0);
      if (
        range.startContainer.nodeType === 3 &&
        el.contains(range.startContainer)
      ) {
        const before = (range.startContainer.textContent ?? "").slice(
          0,
          range.startOffset
        );
        const mentionQ = findActiveMentionTrigger(before);
        const slashQ = mentionQ === null ? findActiveSlashTrigger(before) : null;
        onMentionQuery(mentionQ);
        onSlashQuery?.(slashQ);
      } else {
        onMentionQuery(null);
        onSlashQuery?.(null);
      }
    }, [onChange, onMentionQuery, onSlashQuery]);

    const replaceTriggerWithPill = useCallback(
      (kind: "mention" | "slash", token: string, trailingText = " ") => {
        const el = editorRef.current;
        if (!el) return;
        const doc = el.ownerDocument;
        const sel = window.getSelection();
        const pill =
          kind === "mention"
            ? makePillElement(token, doc)
            : makeSlashPillElement(
                token,
                slashCommands.get(token.toLowerCase()) ?? "workflow",
                doc
              );
        const spaceNode = doc.createTextNode(trailingText);

        let inserted = false;
        if (sel && sel.rangeCount > 0) {
          const range = sel.getRangeAt(0);
          if (
            range.startContainer.nodeType === 3 &&
            el.contains(range.startContainer)
          ) {
            const textNode = range.startContainer as Text;
            const before = textNode.data.slice(0, range.startOffset);
            const triggerMatch =
              kind === "mention"
                ? before.match(/(^|\s)@([a-zA-Z0-9_-]*)$/)
                : before.match(/(^|\s)\/([a-zA-Z0-9_-]*)$/);
            if (triggerMatch) {
              const prefixLen = triggerMatch[1].length;
              const startIdx = before.length - triggerMatch[0].length + prefixLen;
              const leftStr = before.slice(0, startIdx);
              const rightStr = textNode.data.slice(range.startOffset);
              const parent = textNode.parentNode;
              if (parent) {
                const afterNode = doc.createTextNode(
                  rightStr.startsWith(" ") || !trailingText
                    ? rightStr
                    : `${trailingText}${rightStr}`
                );
                if (leftStr) {
                  textNode.data = leftStr;
                  parent.insertBefore(pill, textNode.nextSibling);
                } else {
                  parent.replaceChild(pill, textNode);
                }
                parent.insertBefore(afterNode, pill.nextSibling);
                const newRange = doc.createRange();
                const caretOffset =
                  afterNode.data.startsWith(" ") && trailingText === " " ? 1 : 0;
                newRange.setStart(afterNode, caretOffset);
                newRange.collapse(true);
                sel.removeAllRanges();
                sel.addRange(newRange);
                inserted = true;
              }
            }
          }
        }

        if (!inserted) {
          el.appendChild(pill);
          el.appendChild(spaceNode);
          const newRange = doc.createRange();
          newRange.setStart(spaceNode, spaceNode.data.length);
          newRange.collapse(true);
          sel?.removeAllRanges();
          sel?.addRange(newRange);
        }

        el.focus();
        emitCurrent();
        onMentionQuery(null);
        onSlashQuery?.(null);
      },
      [emitCurrent, onMentionQuery, onSlashQuery, slashCommands]
    );

    useImperativeHandle(
      ref,
      () => ({
        focus: () => editorRef.current?.focus(),
        insertMention: (handle: string) => replaceTriggerWithPill("mention", handle, " "),
        insertSlash: (command: string) => replaceTriggerWithPill("slash", command, " "),
      }),
      [replaceTriggerWithPill]
    );

    const handleClick = (e: React.MouseEvent<HTMLDivElement>) => {
      const target = e.target as HTMLElement;
      const removeMention = target.closest<HTMLElement>("[data-remove-mention]");
      const removeSlash = target.closest<HTMLElement>("[data-remove-slash]");
      if (removeMention || removeSlash) {
        e.preventDefault();
        e.stopPropagation();
        const pill = target.closest<HTMLElement>("[data-mention], [data-slash]");
        if (pill) {
          pill.remove();
          emitCurrent();
          editorRef.current?.focus();
        }
      }
    };

    const handleLocalKeyDown = (e: React.KeyboardEvent<HTMLDivElement>) => {
      const el = editorRef.current;
      const isPill = (n: HTMLElement | undefined | null) =>
        Boolean(n?.dataset?.mention || n?.dataset?.slash);

      if (el && (e.key === "Backspace" || e.key === "Delete")) {
        const sel = window.getSelection();
        if (sel && sel.rangeCount > 0 && sel.isCollapsed) {
          const range = sel.getRangeAt(0);
          let targetPill: HTMLElement | null = null;
          if (e.key === "Backspace") {
            if (range.startContainer === el && range.startOffset > 0) {
              const prev = el.childNodes[range.startOffset - 1] as HTMLElement | undefined;
              if (isPill(prev)) targetPill = prev ?? null;
            } else if (
              range.startContainer.nodeType === 3 &&
              range.startOffset === 0
            ) {
              const prev = range.startContainer.previousSibling as HTMLElement | null;
              if (isPill(prev)) targetPill = prev;
            }
          } else if (e.key === "Delete") {
            if (range.startContainer === el && range.startOffset < el.childNodes.length) {
              const next = el.childNodes[range.startOffset] as HTMLElement | undefined;
              if (isPill(next)) targetPill = next ?? null;
            } else if (
              range.startContainer.nodeType === 3 &&
              range.startOffset === (range.startContainer.textContent?.length ?? 0)
            ) {
              const next = range.startContainer.nextSibling as HTMLElement | null;
              if (isPill(next)) targetPill = next;
            }
          }
          if (targetPill) {
            e.preventDefault();
            targetPill.remove();
            emitCurrent();
            return;
          }
        }
      }

      if (e.key === " " && el) {
        const sel = window.getSelection();
        if (sel && sel.rangeCount > 0 && sel.isCollapsed) {
          const range = sel.getRangeAt(0);
          if (range.startContainer.nodeType === 3 && el.contains(range.startContainer)) {
            const before = (range.startContainer.textContent ?? "").slice(
              0,
              range.startOffset
            );
            const trig = findActiveMentionTrigger(before);
            if (trig) {
              const matched = Array.from(validHandles).find(
                (h) => h.toLowerCase() === trig
              );
              if (matched) {
                e.preventDefault();
                replaceTriggerWithPill("mention", matched, " ");
                return;
              }
            }
            const slashTrig = findActiveSlashTrigger(before);
            if (slashTrig && slashCommands.has(slashTrig)) {
              e.preventDefault();
              replaceTriggerWithPill("slash", slashTrig, " ");
              return;
            }
          }
        }
      }

      onKeyDown?.(e);
    };

    const handlePaste = (e: React.ClipboardEvent<HTMLDivElement>) => {
      e.preventDefault();
      const plain = e.clipboardData.getData("text/plain");
      if (!plain || !editorRef.current) return;
      const doc = editorRef.current.ownerDocument;
      const sel = window.getSelection();
      if (sel && sel.rangeCount > 0) {
        const range = sel.getRangeAt(0);
        range.deleteContents();
        const frag = buildFragmentFromValue(plain, validHandles, slashCommands, doc);
        const lastNode = frag.lastChild;
        range.insertNode(frag);
        if (lastNode) {
          const after = doc.createRange();
          after.setStartAfter(lastNode);
          after.collapse(true);
          sel.removeAllRanges();
          sel.addRange(after);
        }
      } else {
        editorRef.current.appendChild(
          buildFragmentFromValue(plain, validHandles, slashCommands, doc)
        );
      }
      emitCurrent();
    };

    const isEmpty = value.length === 0;

    return (
      <div className="relative min-h-[46px] max-h-[180px] overflow-y-auto">
        {isEmpty && placeholder && (
          <div
            aria-hidden="true"
            className="pointer-events-none absolute inset-0 text-[13.5px] leading-relaxed text-ink-muted select-none"
          >
            {placeholder}
          </div>
        )}
        <div
          ref={editorRef}
          role="textbox"
          aria-multiline="true"
          aria-label="Message group chat"
          contentEditable={!disabled}
          suppressContentEditableWarning
          onInput={emitCurrent}
          onClick={handleClick}
          onKeyDown={handleLocalKeyDown}
          onPaste={handlePaste}
          className="min-h-[46px] w-full bg-transparent text-[13.5px] leading-relaxed text-ink focus:outline-none whitespace-pre-wrap break-words"
        />
      </div>
    );
  }
);
