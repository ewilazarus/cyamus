//! Name normalization: slugs, environment variable names and project names.

/// Maximum length of a DNS label, and therefore of a project name.
pub const MAX_PROJECT_NAME_LEN: usize = 63;

/// Lowercases ASCII and collapses every run of non-`[a-z0-9]` characters into
/// `sep`, trimming `sep` from both ends.
pub fn slugify(raw: &str, sep: char) -> String {
    let mut out = String::with_capacity(raw.len());
    let mut pending_sep = false;
    for c in raw.chars() {
        let c = c.to_ascii_lowercase();
        if c.is_ascii_lowercase() || c.is_ascii_digit() {
            if pending_sep && !out.is_empty() {
                out.push(sep);
            }
            pending_sep = false;
            out.push(c);
        } else {
            pending_sep = true;
        }
    }
    out
}

/// Uppercases and replaces every run of non-alphanumeric characters with `_`,
/// producing the suffix of a `CYAMUS_VAR_*` / `CYAMUS_FINGERPRINT_*` variable.
pub fn env_var_suffix(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    let mut in_run = false;
    for c in raw.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_uppercase());
            in_run = false;
        } else if !in_run {
            out.push('_');
            in_run = true;
        }
    }
    out
}

/// Normalizes a guessed name into a valid project name, or `None` if nothing
/// usable remains.
pub fn project_name_from(raw: &str) -> Option<String> {
    let mut slug = slugify(raw, '-');
    slug.truncate(MAX_PROJECT_NAME_LEN);
    let slug = slug.trim_end_matches('-').to_owned();
    (!slug.is_empty()).then_some(slug)
}

/// Checks that `name` is a DNS-label-safe project name.
pub fn validate_project_name(name: &str) -> Result<(), String> {
    let rule = "project names must be 1-63 characters of lowercase letters, digits and hyphens, \
                not starting or ending with a hyphen";
    let valid = !name.is_empty()
        && name.len() <= MAX_PROJECT_NAME_LEN
        && !name.starts_with('-')
        && !name.ends_with('-')
        && name
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-');
    if valid {
        Ok(())
    } else {
        Err(format!("invalid project name {name:?}: {rule}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slugs() {
        assert_eq!(slugify("feature/My-Thing", '-'), "feature-my-thing");
        assert_eq!(slugify("feature/My-Thing", '_'), "feature_my_thing");
        assert_eq!(slugify("--a__b--", '-'), "a-b");
        assert_eq!(slugify("Ünïcode", '-'), "n-code");
        assert_eq!(slugify("///", '-'), "");
    }

    #[test]
    fn env_suffixes() {
        assert_eq!(env_var_suffix("node-version"), "NODE_VERSION");
        assert_eq!(env_var_suffix("node_version"), "NODE_VERSION");
        assert_eq!(env_var_suffix("docker-deps"), "DOCKER_DEPS");
        assert_eq!(env_var_suffix("a..b"), "A_B");
    }

    #[test]
    fn project_names() {
        assert_eq!(project_name_from("My_Proj").as_deref(), Some("my-proj"));
        assert_eq!(project_name_from("widgets").as_deref(), Some("widgets"));
        assert_eq!(project_name_from("!!!"), None);
        let long = format!("{}-{}", "a".repeat(62), "b");
        let name = project_name_from(&long).unwrap();
        assert_eq!(name, "a".repeat(62));
        assert!(validate_project_name(&name).is_ok());
    }

    #[test]
    fn validation() {
        assert!(validate_project_name("my-proj").is_ok());
        assert!(validate_project_name("My Proj!").is_err());
        assert!(validate_project_name("-a").is_err());
        assert!(validate_project_name("a-").is_err());
        assert!(validate_project_name("").is_err());
        assert!(validate_project_name(&"a".repeat(64)).is_err());
    }
}
