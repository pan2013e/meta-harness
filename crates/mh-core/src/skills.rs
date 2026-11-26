use std::path::{Component, Path, PathBuf};

use serde::{Deserialize, Serialize};

/// Built-in composer slash commands that cannot be shadowed by installed skill slugs.
pub const RESERVED_SLASH_COMMANDS: &[&str] = &[
    "plan", "review", "test", "fix", "compact", "diff", "shell", "memory", "new", "archive",
];
pub const MAX_SKILL_FILE_BYTES: u64 = 512 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SkillSource {
    Builtin,
    Local,
    Git,
    Custom,
    Project,
}

impl SkillSource {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Builtin => "builtin",
            Self::Local => "local",
            Self::Git => "git",
            Self::Custom => "custom",
            Self::Project => "project",
        }
    }

    pub fn parse(raw: &str) -> Self {
        match raw.trim().to_ascii_lowercase().as_str() {
            "builtin" => Self::Builtin,
            "git" => Self::Git,
            "custom" => Self::Custom,
            "project" => Self::Project,
            _ => Self::Local,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillSummary {
    pub id: String,
    pub name: String,
    pub description: String,
    pub source: SkillSource,
    #[serde(default)]
    pub origin: Option<String>,
    pub enabled: bool,
    #[serde(default)]
    pub builtin_modified: bool,
    pub skill_dir: String,
    pub skill_md_path: String,
    pub files: Vec<String>,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillDetail {
    pub skill: SkillSummary,
    pub file_path: String,
    pub content: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedSkillMeta {
    pub slug: String,
    pub name: String,
    pub description: String,
}

pub fn resolve_skill_file(root: &Path, relative: &str) -> Result<PathBuf, String> {
    let path = Path::new(relative);
    if relative.is_empty()
        || path
            .components()
            .any(|part| !matches!(part, Component::Normal(_)))
    {
        return Err("Skill file must be a relative path inside the skill directory.".into());
    }
    let target = root.join(path);
    if !crate::is_path_within(&target, root)
        || std::fs::symlink_metadata(&target).is_ok_and(|meta| meta.file_type().is_symlink())
    {
        return Err("Skill file must stay inside the skill directory.".into());
    }
    Ok(target)
}

pub fn sanitize_skill_slug(raw: &str) -> Result<String, String> {
    let trimmed = raw
        .trim()
        .trim_start_matches('/')
        .to_ascii_lowercase()
        .replace(' ', "-");
    if trimmed.is_empty() || trimmed.len() > 64 {
        return Err("Skill slug must be 1 to 64 characters long.".into());
    }
    let bytes = trimmed.as_bytes();
    if !bytes[0].is_ascii_alphanumeric() {
        return Err("Skill slug must start with an ASCII letter or digit.".into());
    }
    if !bytes
        .iter()
        .all(|b| b.is_ascii_alphanumeric() || *b == b'-' || *b == b'_')
    {
        return Err(
            "Skill slug may only contain letters, digits, hyphens, and underscores.".into(),
        );
    }
    if RESERVED_SLASH_COMMANDS.contains(&trimmed.as_str()) {
        return Err(format!(
            "'/{trimmed}' is a built-in slash command and cannot be used as a skill slug."
        ));
    }
    Ok(trimmed)
}

pub fn is_reserved_slash_command(slug: &str) -> bool {
    RESERVED_SLASH_COMMANDS
        .iter()
        .any(|reserved| reserved.eq_ignore_ascii_case(slug.trim_start_matches('/')))
}

pub fn humanize_skill_slug(slug: &str) -> String {
    let words: Vec<String> = slug
        .split(['-', '_'])
        .filter(|part| !part.is_empty())
        .map(|part| {
            let mut chars = part.chars();
            match chars.next() {
                Some(first) => {
                    let mut s = first.to_uppercase().to_string();
                    s.push_str(chars.as_str());
                    s
                }
                None => String::new(),
            }
        })
        .collect();
    if words.is_empty() {
        slug.to_string()
    } else {
        words.join(" ")
    }
}

pub fn parse_skill_markdown(raw: &str, fallback_slug: &str) -> Result<ParsedSkillMeta, String> {
    let normalized = raw.strip_prefix('\u{FEFF}').unwrap_or(raw);
    if normalized.trim().is_empty() {
        return Err("SKILL.md cannot be empty.".into());
    }

    let (fm_name, fm_title, fm_desc, body) = parse_frontmatter(normalized);
    let slug = if let Some(candidate) = fm_name.as_deref() {
        sanitize_skill_slug(candidate).unwrap_or_else(|_| fallback_slug.to_ascii_lowercase())
    } else {
        fallback_slug.to_ascii_lowercase()
    };

    let heading_title = body.lines().find_map(|line| {
        let trimmed = line.trim();
        trimmed
            .strip_prefix("# ")
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
    });

    let display_name = fm_title
        .filter(|s| !s.is_empty())
        .or(heading_title)
        .or_else(|| {
            fm_name
                .as_deref()
                .filter(|n| n.chars().any(|c| c.is_ascii_uppercase() || c == ' '))
                .map(str::to_string)
        })
        .unwrap_or_else(|| humanize_skill_slug(&slug));

    let body_summary = body
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty() && !line.starts_with('#') && !line.starts_with("---"))
        .map(|line| truncate_skill_text(line, 240))
        .unwrap_or_default();

    let description = fm_desc
        .filter(|s| !s.trim().is_empty())
        .or_else(|| (!body_summary.is_empty()).then_some(body_summary))
        .unwrap_or_else(|| format!("Instructions for {display_name}."));

    Ok(ParsedSkillMeta {
        slug,
        name: display_name,
        description,
    })
}

pub fn ensure_skill_frontmatter(
    slug: &str,
    name: &str,
    description: &str,
    content: &str,
) -> String {
    let trimmed = content.strip_prefix('\u{FEFF}').unwrap_or(content).trim();
    if trimmed.starts_with("---\n") || trimmed.starts_with("---\r\n") {
        let lines: Vec<&str> = trimmed.lines().collect();
        if lines.iter().skip(1).any(|line| line.trim() == "---") {
            return format!("{trimmed}\n");
        }
    }

    let safe_title = name.replace('"', "'");
    let safe_desc = description.replace('\n', " ").replace('"', "'");
    let body = if trimmed.is_empty() {
        format!("# {name}\n\n{description}\n")
    } else {
        format!("{trimmed}\n")
    };
    format!(
        "---\nname: {slug}\ntitle: \"{safe_title}\"\ndescription: \"{safe_desc}\"\n---\n\n{body}"
    )
}

/// Scans `<dir>/SKILL.md` plus supporting files up to depth 4, returning sorted relative paths
/// (with `SKILL.md` first) and modification timestamp in milliseconds.
pub fn scan_skill_dir(
    dir: &Path,
    fallback_slug: &str,
) -> Result<(ParsedSkillMeta, Vec<String>, i64), String> {
    let skill_md = dir.join("SKILL.md");
    let meta_fs = std::fs::symlink_metadata(&skill_md)
        .map_err(|e| format!("Failed to stat {}: {e}", skill_md.display()))?;
    if !meta_fs.is_file() || !crate::is_path_within(&skill_md, dir) {
        return Err(format!(
            "SKILL.md must be a regular file in {}",
            dir.display()
        ));
    }
    if meta_fs.len() > MAX_SKILL_FILE_BYTES {
        return Err("SKILL.md exceeds 512 KiB size limit.".into());
    }
    let updated_at = meta_fs
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0);

    let raw = std::fs::read_to_string(&skill_md)
        .map_err(|e| format!("Failed to read {}: {e}", skill_md.display()))?;
    let parsed = parse_skill_markdown(&raw, fallback_slug)?;

    let mut rel_files = vec!["SKILL.md".to_string()];
    collect_skill_files(dir, dir, 0, &mut rel_files);
    rel_files[1..].sort();

    Ok((parsed, rel_files, updated_at))
}

/// Computes an OS-independent content hash over normalized `(path, LF content)` pairs
/// so built-in skill update detection matches identically across CRLF and LF checkouts.
pub fn compute_skill_tree_hash(files: &[(&str, &str)]) -> String {
    let mut sorted: Vec<(&str, &str)> = files.to_vec();
    sorted.sort_by(|a, b| a.0.cmp(b.0));

    let mut hash: u128 = 0x6c62_272e_07bb_0142_62b8_2175_6295_c58d;
    const PRIME: u128 = 0x0000_0000_0100_0000_0000_0000_0000_013b;
    let feed = |state: &mut u128, bytes: &[u8]| {
        for &b in bytes {
            *state ^= u128::from(b);
            *state = state.wrapping_mul(PRIME);
        }
    };

    for (rel, content) in sorted {
        let norm_path = rel.replace('\\', "/");
        feed(&mut hash, norm_path.as_bytes());
        feed(&mut hash, b"\0");
        let norm_content = content.replace("\r\n", "\n");
        feed(&mut hash, norm_content.as_bytes());
        feed(&mut hash, b"\n--mh-file--\n");
    }

    format!("{hash:032x}")
}

fn collect_skill_files(root: &Path, current: &Path, depth: usize, out: &mut Vec<String>) {
    if depth > 4 || out.len() >= 120 {
        return;
    }
    let Ok(rd) = std::fs::read_dir(current) else {
        return;
    };
    let mut entries: Vec<_> = rd.flatten().collect();
    entries.sort_by_key(|e| e.file_name());
    for entry in entries {
        if out.len() >= 120 {
            break;
        }
        let name = entry.file_name();
        let name_str = name.to_string_lossy();
        if name_str.starts_with('.') || name_str == "node_modules" {
            continue;
        }
        let Ok(ft) = entry.file_type() else {
            continue;
        };
        let path = entry.path();
        if ft.is_symlink() || path == root.join("SKILL.md") || !crate::is_path_within(&path, root) {
            continue;
        }
        if ft.is_dir() {
            collect_skill_files(root, &path, depth + 1, out);
        } else if ft.is_file()
            && let Ok(rel) = path.strip_prefix(root)
        {
            out.push(rel.to_string_lossy().replace('\\', "/"));
        }
    }
}

fn parse_frontmatter(raw: &str) -> (Option<String>, Option<String>, Option<String>, &str) {
    let mut lines = raw.split_inclusive('\n');
    let Some(first) = lines.next() else {
        return (None, None, None, raw);
    };
    if first.trim() != "---" {
        return (None, None, None, raw);
    }

    let mut offset = first.len();
    let mut fm_end = None;
    let mut fm_lines = Vec::new();
    for line in lines {
        offset += line.len();
        if line.trim() == "---" {
            fm_end = Some(offset);
            break;
        }
        fm_lines.push(line.trim_end_matches(['\r', '\n']));
    }
    let Some(body_start) = fm_end else {
        return (None, None, None, raw);
    };

    let mut name = None;
    let mut title = None;
    let mut desc = None;
    let mut idx = 0usize;

    while idx < fm_lines.len() {
        let line = fm_lines[idx];
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            idx += 1;
            continue;
        }
        if let Some((k, v)) = line.split_once(':') {
            let key = k.trim().to_ascii_lowercase();
            let raw_val = v.trim();
            if matches!(raw_val, ">" | ">-" | "|" | "|-") {
                let mut block = Vec::new();
                idx += 1;
                while idx < fm_lines.len() {
                    let sub = fm_lines[idx];
                    if sub.starts_with(' ') || sub.starts_with('\t') || sub.trim().is_empty() {
                        block.push(sub.trim());
                        idx += 1;
                    } else {
                        break;
                    }
                }
                let joined = block
                    .into_iter()
                    .filter(|s| !s.is_empty())
                    .collect::<Vec<_>>()
                    .join(" ");
                match key.as_str() {
                    "name" => name = Some(joined),
                    "title" => title = Some(joined),
                    "description" => desc = Some(joined),
                    _ => {}
                }
                continue;
            }
            let unquoted = strip_quotes(raw_val);
            match key.as_str() {
                "name" => name = Some(unquoted),
                "title" => title = Some(unquoted),
                "description" => desc = Some(unquoted),
                _ => {}
            }
        }
        idx += 1;
    }

    (name, title, desc, &raw[body_start..])
}

fn strip_quotes(raw: &str) -> String {
    let s = raw.trim();
    if s.len() >= 2
        && ((s.starts_with('"') && s.ends_with('"')) || (s.starts_with('\'') && s.ends_with('\'')))
    {
        s[1..s.len() - 1].trim().to_string()
    } else {
        s.to_string()
    }
}

fn truncate_skill_text(s: &str, max_chars: usize) -> String {
    if s.chars().count() <= max_chars {
        s.to_string()
    } else {
        let mut out: String = s.chars().take(max_chars).collect();
        out.push_str("...");
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitize_rejects_reserved_and_accepts_skill_slugs() {
        assert_eq!(
            sanitize_skill_slug("/Meta-Harness Docs").unwrap(),
            "meta-harness-docs"
        );
        assert!(sanitize_skill_slug("plan").is_err());
        assert!(sanitize_skill_slug("memory").is_err());
        assert!(sanitize_skill_slug("../evil").is_err());
    }

    #[test]
    fn parses_frontmatter_and_multiline_descriptions() {
        let md = "---\nname: security-audit\ntitle: \"Security Audit\"\ndescription: >\n  Review authentication,\n  worktrees, and CSP.\n---\n\n# Ignored Fallback\n";
        let meta = parse_skill_markdown(md, "fallback").unwrap();
        assert_eq!(meta.slug, "security-audit");
        assert_eq!(meta.name, "Security Audit");
        assert_eq!(
            meta.description,
            "Review authentication, worktrees, and CSP."
        );
    }

    #[test]
    fn hash_ignores_crlf_vs_lf_and_input_order() {
        let a = compute_skill_tree_hash(&[
            ("SKILL.md", "line1\r\nline2\r\n"),
            ("references/a.md", "hello\r\n"),
        ]);
        let b = compute_skill_tree_hash(&[
            ("references/a.md", "hello\n"),
            ("SKILL.md", "line1\nline2\n"),
        ]);
        assert_eq!(a, b);
    }
}
