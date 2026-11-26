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

pub struct UserMentionRouting {
    pub mentions: ParsedMentions,
    pub needs_intent: bool,
}

impl ParsedMentions {
    pub fn is_unmentioned(&self) -> bool {
        self.targets.is_empty() && !self.has_all && !self.has_orchestrator
    }
}

/// Masks blockquotes, fenced code blocks (```` ``` ```` or `~~~`) and inline code spans (`` `...` ``)
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

        if trimmed.starts_with('>') {
            push_masked_line(&mut after_fences, line);
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

/// Extracts all valid mentions outside quotes and code. Agents use this
/// when handing off a completed reply or naming peers in an MCP message.
pub fn parse_mentions(markdown: &str, valid_handles: &HashSet<String>) -> ParsedMentions {
    parse_clean_mentions(&mask_code_regions(markdown), valid_handles)
}

/// A leading recipient list addresses the initial turn. Mentions in the
/// following instruction are still passed to those recipients as prompt text.
pub fn parse_user_mentions(markdown: &str, valid_handles: &HashSet<String>) -> UserMentionRouting {
    let cleaned = mask_code_regions(markdown);
    let mut start = cleaned.trim_start();
    while let Some(command) = start.split_whitespace().next() {
        if !is_leading_slash_token(command) {
            break;
        }
        start = start[command.len()..].trim_start();
    }

    let mut leading = ParsedMentions::default();
    let mut seen = HashSet::new();
    let mut remaining = start;
    while let Some((mention, tail)) = mention_at_start(remaining) {
        add_target(&mut leading, &mut seen, valid_handles, mention);
        remaining = tail.trim_start_matches(|ch: char| ch.is_whitespace() || ch == ',');
        if remaining.len() == tail.len() {
            break;
        }
        if let Some(word) = remaining.split_whitespace().next()
            && (word.eq_ignore_ascii_case("and") || word == "&")
        {
            remaining = remaining[word.len()..].trim_start();
        }
    }

    if !leading.is_unmentioned() {
        return UserMentionRouting {
            mentions: leading,
            needs_intent: false,
        };
    }

    let mut mentions = ParsedMentions::default();
    let mut previous_end = None;
    let mut separated = false;
    for (index, end, mention) in scan_mentions(&cleaned) {
        let before = mentions.targets.len();
        add_target(&mut mentions, &mut seen, valid_handles, mention);
        if mentions.targets.len() != before {
            separated |= previous_end.is_some_and(|start| {
                let gap = cleaned[start..index]
                    .trim_matches(|c: char| c.is_whitespace() || ",()".contains(c));
                !gap.is_empty() && !gap.eq_ignore_ascii_case("and") && gap != "&"
            });
            previous_end = Some(end);
        }
    }
    let needs_intent = separated && !mentions.has_all && !mentions.has_orchestrator;
    UserMentionRouting {
        mentions,
        needs_intent,
    }
}

fn parse_clean_mentions(cleaned: &str, valid_handles: &HashSet<String>) -> ParsedMentions {
    let mut out = ParsedMentions::default();
    let mut seen = HashSet::new();
    for (_, _, mention) in scan_mentions(cleaned) {
        add_target(&mut out, &mut seen, valid_handles, mention);
    }
    out
}

fn scan_mentions(cleaned: &str) -> impl Iterator<Item = (usize, usize, ParsedMentionTarget)> + '_ {
    let mut next_index = 0;
    let mut previous = None;
    cleaned.char_indices().filter_map(move |(index, ch)| {
        let starts_mention =
            index >= next_index && ch == '@' && previous.is_none_or(is_mention_prefix_boundary);
        previous = Some(ch);
        if starts_mention && let Some((mention, tail)) = mention_at_start(&cleaned[index..]) {
            next_index = cleaned.len() - tail.len();
            Some((index, next_index, mention))
        } else {
            None
        }
    })
}

fn mention_at_start(input: &str) -> Option<(ParsedMentionTarget, &str)> {
    let input = input.strip_prefix('@')?;
    let len = input
        .bytes()
        .take_while(|byte| byte.is_ascii_alphanumeric() || b"_-".contains(byte))
        .count();
    let (handle, mut tail) = input.split_at(len);
    // Reject scoped packages and domain names while allowing sentence punctuation.
    if handle.is_empty()
        || tail.starts_with('/')
        || tail
            .strip_prefix('.')
            .and_then(|rest| rest.as_bytes().first())
            .is_some_and(u8::is_ascii_alphanumeric)
    {
        return None;
    }

    let mut target = ParsedMentionTarget {
        handle: handle.to_string(),
        model_override: None,
        effort_override: None,
    };
    if let Some((qualifier, rest)) = tail.strip_prefix('[').and_then(|tail| tail.split_once(']'))
        && !qualifier.is_empty()
        && !qualifier.contains(char::is_whitespace)
    {
        let (model, effort) = qualifier.split_once(':').unwrap_or((qualifier, ""));
        target.model_override = (!model.is_empty()).then(|| model.to_string());
        target.effort_override = (!effort.is_empty()).then(|| effort.to_string());
        tail = rest;
    }

    Some((target, tail))
}

fn add_target(
    out: &mut ParsedMentions,
    seen: &mut HashSet<String>,
    valid_handles: &HashSet<String>,
    mut target: ParsedMentionTarget,
) {
    if target.handle.eq_ignore_ascii_case("all") {
        out.has_all = true;
    } else if target.handle.eq_ignore_ascii_case("orchestrator") {
        out.has_orchestrator = true;
    } else if let Some(handle) = valid_handles
        .iter()
        .find(|handle| handle.eq_ignore_ascii_case(&target.handle))
        && seen.insert(handle.to_ascii_lowercase())
    {
        target.handle.clone_from(handle);
        out.targets.push(target);
    }
}

/// Extracts distinct `/token` directives outside quotes and code.
pub fn parse_slash_tokens(markdown: &str) -> Vec<String> {
    let cleaned = mask_code_regions(markdown);
    let mut tokens = Vec::new();
    for raw in cleaned.split_whitespace() {
        let stripped = raw
            .trim_start_matches(['(', '[', '{', '"', '\''])
            .trim_end_matches([',', '.', ';', ':', '!', '?', ')', ']', '}', '"', '\'']);
        if is_leading_slash_token(stripped) {
            let name = stripped[1..].to_ascii_lowercase();
            if !tokens.contains(&name) {
                tokens.push(name);
            }
        }
    }
    tokens
}

fn is_leading_slash_token(token: &str) -> bool {
    let name = token.trim_end_matches([',', '.', ';', ':', '!', '?']);
    let Some(rest) = name.strip_prefix('/') else {
        return false;
    };
    !rest.is_empty()
        && rest.len() <= 64
        && rest
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

fn is_mention_prefix_boundary(ch: char) -> bool {
    ch.is_whitespace()
        || matches!(
            ch,
            '(' | '[' | '{' | '"' | '\'' | ',' | ':' | ';' | '>' | '*'
        )
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

    #[test]
    fn separates_leading_recipients_from_body_references() {
        let routing = parse_user_mentions(
            "/plan @codex[gpt-6.2-sol:high] say hello to @antigravity",
            &handles(),
        );
        assert!(!routing.needs_intent);
        assert_eq!(routing.mentions.targets.len(), 1);
        assert_eq!(routing.mentions.targets[0].handle, "codex");
        assert_eq!(
            routing.mentions.targets[0].effort_override.as_deref(),
            Some("high")
        );
        assert_eq!(
            parse_mentions("Hi, @antigravity!", &handles()).targets[0].handle,
            "antigravity"
        );
    }

    #[test]
    fn distinguishes_explicit_lists_from_ambiguous_inline_mentions() {
        for text in [
            "@codex and @antigravity review",
            "Ask (@codex) and @antigravity to review",
        ] {
            let routing = parse_user_mentions(text, &handles());
            assert_eq!(routing.mentions.targets.len(), 2);
            assert!(!routing.needs_intent, "{text}");
        }
        let routing = parse_user_mentions("Ask @codex to greet @antigravity", &handles());
        assert_eq!(routing.mentions.targets.len(), 2);
        assert!(routing.needs_intent);
        let routing = parse_user_mentions(
            "@codex point out `@all` and ask about @antigravity",
            &handles(),
        );
        assert_eq!(routing.mentions.targets.len(), 1);
        assert!(!routing.mentions.has_all);

        let skill_routing = parse_user_mentions(
            "/meta-harness-docs /plan @codex inspect `/ignored-skill` and explain routing",
            &handles(),
        );
        assert!(!skill_routing.needs_intent);
        assert_eq!(skill_routing.mentions.targets.len(), 1);
        assert_eq!(skill_routing.mentions.targets[0].handle, "codex");

        let slash = parse_slash_tokens(
            "/meta-harness-docs compare with `/code-span` and /plan\n```\n/not-a-skill\n```",
        );
        assert_eq!(slash, vec!["meta-harness-docs", "plan"]);
    }

    #[test]
    fn quoted_messages_do_not_route_or_activate_skills() {
        let draft = "> @claude /meta-harness-docs\n> @all\n\n@codex /review Check the quoted reply";
        let routing = parse_user_mentions(draft, &handles());
        assert!(!routing.needs_intent && !routing.mentions.has_all);
        assert_eq!(routing.mentions.targets.len(), 1);
        assert_eq!(routing.mentions.targets[0].handle, "codex");
        assert_eq!(parse_slash_tokens(draft), ["review"]);
        assert!(parse_mentions("> @claude referenced a peer", &handles()).is_unmentioned());
    }
}
