use std::collections::HashSet;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedMentionTarget {
    pub handle: String,
    pub model_override: Option<String>,
    pub effort_override: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ParsedMentions {
    pub targets: Vec<ParsedMentionTarget>,
    pub has_all: bool,
    pub has_orchestrator: bool,
}

impl ParsedMentions {
    pub fn is_unmentioned(&self) -> bool {
        self.targets.is_empty() && !self.has_all && !self.has_orchestrator
    }
}

/// Masks fenced code blocks (```` ``` ```` or `~~~`) and inline code spans (`` `...` ``)
/// with ASCII spaces while leaving surrounding markdown untouched.
pub fn mask_code_regions(markdown: &str) -> String {
    let lines: Vec<&str> = markdown.split_inclusive('\n').collect();
    let mut after_fences = String::with_capacity(markdown.len());

    let mut active_fence: Option<(char, usize)> = None;
    for line in lines {
        let trimmed = line.trim_start_matches([' ', '\t']);
        if let Some((fence_ch, fence_len)) = active_fence {
            // Closing fence: same char repeated >= fence_len, only optional whitespace after
            let count = trimmed.chars().take_while(|&c| c == fence_ch).count();
            let rest_blank = trimmed[count..]
                .trim_matches([' ', '\t', '\r', '\n'])
                .is_empty();
            push_masked_line(&mut after_fences, line);
            if count >= fence_len && rest_blank {
                active_fence = None;
            }
            continue;
        }

        let first = trimmed.chars().next();
        if matches!(first, Some('`') | Some('~')) {
            let fence_ch = first.unwrap_or('`');
            let count = trimmed.chars().take_while(|&c| c == fence_ch).count();
            if count >= 3 {
                // Backtick info strings cannot contain backticks in CommonMark
                let info = &trimmed[count..];
                let valid_fence = fence_ch != '`' || !info.contains('`');
                if valid_fence {
                    active_fence = Some((fence_ch, count));
                    push_masked_line(&mut after_fences, line);
                    continue;
                }
            }
        }

        after_fences.push_str(line);
    }

    // Mask inline code spans (`...`)
    let mut result = String::with_capacity(after_fences.len());
    let chars: Vec<char> = after_fences.chars().collect();
    let mut i = 0usize;
    while i < chars.len() {
        if chars[i] == '`' {
            let mut open_len = 0usize;
            while i + open_len < chars.len() && chars[i + open_len] == '`' {
                open_len += 1;
            }
            // Look ahead for matching backtick sequence of exact length `open_len`
            let mut j = i + open_len;
            let mut closed_at = None;
            while j < chars.len() {
                if chars[j] == '`' {
                    let mut run = 0usize;
                    while j + run < chars.len() && chars[j + run] == '`' {
                        run += 1;
                    }
                    if run == open_len {
                        closed_at = Some(j + run);
                        break;
                    }
                    j += run;
                } else {
                    j += 1;
                }
            }

            if let Some(end) = closed_at {
                for ch in &chars[i..end] {
                    if *ch == '\n' || *ch == '\r' {
                        result.push(*ch);
                    } else {
                        result.push(' ');
                    }
                }
                i = end;
                continue;
            }
        }
        result.push(chars[i]);
        i += 1;
    }

    result
}

fn push_masked_line(out: &mut String, line: &str) {
    for ch in line.chars() {
        if ch == '\n' || ch == '\r' {
            out.push(ch);
        } else {
            out.push(' ');
        }
    }
}

/// Extracts valid `@handle`, `@handle[model:effort]`, `@all`, and `@orchestrator` mentions
/// outside fenced and inline code regions.
pub fn parse_mentions(markdown: &str, valid_handles: &HashSet<String>) -> ParsedMentions {
    let cleaned = mask_code_regions(markdown);
    let mut out = ParsedMentions::default();
    let mut seen_handles: HashSet<String> = HashSet::new();

    let chars: Vec<char> = cleaned.chars().collect();
    let mut i = 0usize;

    while i < chars.len() {
        if chars[i] == '@' {
            let prev_ok = i == 0 || is_mention_prefix_boundary(chars[i - 1]);
            if prev_ok {
                let start = i + 1;
                let mut end = start;
                while end < chars.len() && is_handle_char(chars[end]) {
                    end += 1;
                }
                if end > start {
                    // Ensure handle isn't a scoped npm package (`@scope/pkg`) or domain (`@foo.com`),
                    // while allowing trailing sentence punctuation (`@all.`, `@codex,`).
                    let after_ok = if end == chars.len() {
                        true
                    } else if chars[end] == '/' {
                        false
                    } else if chars[end] == '.' {
                        !chars
                            .get(end + 1)
                            .is_some_and(|c| c.is_ascii_alphanumeric())
                    } else {
                        !is_handle_char(chars[end])
                    };
                    if after_ok {
                        let raw_handle: String = chars[start..end].iter().collect();
                        let handle_lc = raw_handle.to_ascii_lowercase();

                        let mut model_override = None;
                        let mut effort_override = None;
                        let mut next_idx = end;

                        // Optional `[model]` or `[model:effort]`
                        if end < chars.len()
                            && chars[end] == '['
                            && let Some(close_rel) = chars[end + 1..].iter().position(|&c| c == ']')
                        {
                            let bracket_str: String =
                                chars[end + 1..end + 1 + close_rel].iter().collect();
                            if !bracket_str.is_empty() && !bracket_str.contains(char::is_whitespace)
                            {
                                if let Some((m, e)) = bracket_str.split_once(':') {
                                    if !m.is_empty() {
                                        model_override = Some(m.to_string());
                                    }
                                    if !e.is_empty() {
                                        effort_override = Some(e.to_string());
                                    }
                                } else {
                                    model_override = Some(bracket_str);
                                }
                                next_idx = end + 1 + close_rel + 1;
                            }
                        }

                        if handle_lc == "all" {
                            out.has_all = true;
                        } else if handle_lc == "orchestrator" {
                            out.has_orchestrator = true;
                        } else if let Some(matched) = valid_handles
                            .iter()
                            .find(|h| h.eq_ignore_ascii_case(&raw_handle))
                        {
                            let key = matched.to_ascii_lowercase();
                            if seen_handles.insert(key) {
                                out.targets.push(ParsedMentionTarget {
                                    handle: matched.clone(),
                                    model_override,
                                    effort_override,
                                });
                            }
                        }

                        i = next_idx;
                        continue;
                    }
                }
            }
        }
        i += 1;
    }

    out
}

fn is_mention_prefix_boundary(ch: char) -> bool {
    ch.is_whitespace()
        || matches!(
            ch,
            '(' | '[' | '{' | '"' | '\'' | ',' | ':' | ';' | '>' | '*'
        )
}

fn is_handle_char(ch: char) -> bool {
    ch.is_ascii_alphanumeric() || ch == '_' || ch == '-'
}

#[cfg(test)]
mod tests {
    use super::*;

    fn handles() -> HashSet<String> {
        ["codex", "claude", "antigravity", "codex-reviewer"]
            .into_iter()
            .map(str::to_string)
            .collect()
    }

    #[test]
    fn extracts_valid_mentions_and_model_effort_overrides() {
        let parsed = parse_mentions(
            "Hey @codex[gpt-5.4:high] and (@claude), check with @codex-reviewer please.",
            &handles(),
        );
        assert!(!parsed.is_unmentioned());
        assert_eq!(
            parsed.targets,
            vec![
                ParsedMentionTarget {
                    handle: "codex".into(),
                    model_override: Some("gpt-5.4".into()),
                    effort_override: Some("high".into()),
                },
                ParsedMentionTarget {
                    handle: "claude".into(),
                    model_override: None,
                    effort_override: None,
                },
                ParsedMentionTarget {
                    handle: "codex-reviewer".into(),
                    model_override: None,
                    effort_override: None,
                },
            ]
        );
    }

    #[test]
    fn ignores_code_blocks_inline_code_and_emails() {
        let md = r#"
Please review `user@codex.com` and `@claude` inline.
Also test email dev@codex.io and package @codex/sdk.
```python
@codex
def handler():
    pass
```
~~~
@antigravity inside tilde fence
~~~
Real ping: @antigravity
"#;
        let parsed = parse_mentions(md, &handles());
        assert_eq!(
            parsed.targets,
            vec![ParsedMentionTarget {
                handle: "antigravity".into(),
                model_override: None,
                effort_override: None,
            }]
        );
    }

    #[test]
    fn recognizes_reserved_all_and_orchestrator() {
        let p = parse_mentions("@orchestrator summarize, or ask @all.", &handles());
        assert!(p.has_orchestrator);
        assert!(p.has_all);
        assert!(p.targets.is_empty());
    }
}
