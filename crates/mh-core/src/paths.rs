use std::path::{Component, Path, PathBuf};

/// Resolved platform-standard configuration and data directories for Meta-Harness.
#[derive(Debug, Clone)]
pub struct AppPaths {
    pub config_dir: PathBuf,
    pub data_dir: PathBuf,
}

impl AppPaths {
    /// Resolves OS-appropriate directories on Windows (`%APPDATA%`/`%LOCALAPPDATA%`),
    /// macOS (`~/Library/Application Support`), and Linux (`$XDG_CONFIG_HOME`/`$XDG_DATA_HOME`),
    /// honoring `MH_CONFIG_DIR` and `MH_DATA_DIR` environment overrides.
    pub fn resolve() -> Self {
        let config_dir = std::env::var_os("MH_CONFIG_DIR")
            .filter(|v| !v.is_empty())
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                dirs::config_dir()
                    .unwrap_or_else(|| PathBuf::from("."))
                    .join("meta-harness")
            });

        let data_dir = std::env::var_os("MH_DATA_DIR")
            .filter(|v| !v.is_empty())
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                dirs::data_local_dir()
                    .or_else(dirs::data_dir)
                    .unwrap_or_else(|| PathBuf::from(".meta-harness"))
                    .join("meta-harness")
            });

        Self {
            config_dir,
            data_dir,
        }
    }

    pub fn db_path(&self) -> PathBuf {
        self.data_dir.join("meta-harness.db")
    }
}

/// Normalizes and expands user-supplied path strings across Windows, macOS, and Linux:
/// trims whitespace, strips matching surrounding single/double quotes (e.g. from Explorer "Copy as path"),
/// expands leading `~` to the user's home directory, and normalizes bare Windows drive specs like `C:` -> `C:\`.
pub fn expand_user_path(raw: &str) -> PathBuf {
    let mut s = raw.trim();
    if s.len() >= 2
        && ((s.starts_with('"') && s.ends_with('"')) || (s.starts_with('\'') && s.ends_with('\'')))
    {
        s = s[1..s.len() - 1].trim();
    }

    if s == "~"
        && let Some(home) = dirs::home_dir()
    {
        return dunce::simplified(&home).to_path_buf();
    }

    if let Some(rest) = s.strip_prefix("~/").or_else(|| s.strip_prefix("~\\"))
        && let Some(home) = dirs::home_dir()
    {
        let cleaned = rest.trim_start_matches(['/', '\\']);
        let joined = if cleaned.is_empty() {
            home
        } else {
            home.join(cleaned)
        };
        return dunce::simplified(&joined).to_path_buf();
    }

    #[cfg(windows)]
    if s.len() == 2 && s.as_bytes()[0].is_ascii_alphabetic() && s.as_bytes()[1] == b':' {
        return PathBuf::from(format!("{s}\\"));
    }

    PathBuf::from(s)
}

/// Canonicalizes an existing path without adding Windows verbatim `\\?\` prefixes
/// and resolving OS symlinks (such as `/var` -> `/private/var` on macOS).
pub fn normalize_path(path: &Path) -> std::io::Result<PathBuf> {
    dunce::canonicalize(path)
}

/// Normalizes a target path that may not yet exist on disk by canonicalizing its
/// deepest existing ancestor and lexically normalizing the remaining suffix.
pub fn normalize_for_create(path: &Path) -> std::io::Result<PathBuf> {
    if path.exists() {
        return dunce::canonicalize(path);
    }

    let abs = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()?.join(path)
    };

    let mut tail_parts: Vec<std::ffi::OsString> = Vec::new();
    let mut cursor = abs.as_path();

    while !cursor.exists() {
        if let Some(name) = cursor.file_name() {
            tail_parts.push(name.to_os_string());
        } else {
            break;
        }
        match cursor.parent() {
            Some(parent) => cursor = parent,
            None => break,
        }
    }

    let mut base = if cursor.exists() {
        dunce::canonicalize(cursor)?
    } else {
        lexical_clean(&abs)
    };

    for part in tail_parts.into_iter().rev() {
        let s = part.to_string_lossy();
        if s == "." || s.is_empty() {
            continue;
        } else if s == ".." {
            base.pop();
        } else {
            base.push(part);
        }
    }

    Ok(dunce::simplified(&base).to_path_buf())
}

/// Checks whether `candidate` resolves strictly inside `root` (or equals `root`),
/// preventing `..` traversal and symlink escape on Windows, macOS, and Linux.
pub fn is_path_within(candidate: &Path, root: &Path) -> bool {
    let Ok(norm_root) = dunce::canonicalize(root) else {
        return false;
    };
    let Ok(norm_candidate) = normalize_for_create(candidate) else {
        return false;
    };

    #[cfg(windows)]
    {
        let c = norm_candidate.to_string_lossy().to_ascii_lowercase();
        let r = norm_root.to_string_lossy().to_ascii_lowercase();
        let c_trimmed = c.trim_end_matches(['\\', '/']);
        let r_trimmed = r.trim_end_matches(['\\', '/']);
        if c_trimmed == r_trimmed {
            return true;
        }
        let prefix = format!("{r_trimmed}\\");
        let prefix_slash = format!("{r_trimmed}/");
        c_trimmed.starts_with(&prefix) || c_trimmed.starts_with(&prefix_slash)
    }

    #[cfg(not(windows))]
    {
        norm_candidate.starts_with(&norm_root)
    }
}

fn lexical_clean(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for comp in path.components() {
        match comp {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn path_containment_allows_inside_and_blocks_traversal() {
        let tmp = std::env::temp_dir().join(format!("mh_path_test_{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(tmp.join("sub")).unwrap();

        let inside_existing = tmp.join("sub");
        let inside_new = tmp.join("sub").join("new_dir").join("file.rs");
        let outside = tmp.join("sub").join("..").join("..").join("escape.txt");

        assert!(is_path_within(&inside_existing, &tmp));
        assert!(is_path_within(&inside_new, &tmp));
        assert!(!is_path_within(&outside, &tmp));

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn expand_user_path_strips_quotes_and_expands_tilde() {
        let quoted = expand_user_path("  \"/tmp/demo\"  ");
        assert_eq!(quoted, PathBuf::from("/tmp/demo"));

        if let Some(home) = dirs::home_dir() {
            let expanded_home = expand_user_path("~");
            assert_eq!(expanded_home, dunce::simplified(&home));

            let sub = expand_user_path("~/projects/app");
            assert_eq!(sub, dunce::simplified(&home.join("projects/app")));
        }

        #[cfg(windows)]
        {
            assert_eq!(expand_user_path("C:"), PathBuf::from("C:\\"));
            assert_eq!(
                expand_user_path("\"C:\\Users\\demo\\repo\""),
                PathBuf::from("C:\\Users\\demo\\repo")
            );
        }
    }
}
