//! Splitting `cyamus compose` arguments into compose's global options and the
//! subcommand, so cyamus can add its own `-p`/`-f` in the right place.

/// Global options of `docker compose` that take a value.
const VALUE_OPTIONS: &[&str] = &[
    "-f",
    "--file",
    "-p",
    "--project-name",
    "--profile",
    "--env-file",
    "--project-directory",
    "--ansi",
    "--progress",
    "--parallel",
];

#[derive(Debug, Default, PartialEq, Eq)]
pub struct Split {
    /// Global options before the subcommand, exactly as given.
    pub globals: Vec<String>,
    /// The user passed `-f`/`--file`.
    pub has_files: bool,
    /// The user passed `-p`/`--project-name`.
    pub has_project: bool,
    /// The compose subcommand (`up`, `down`, ...), if any.
    pub subcommand: Option<String>,
    /// Everything after the subcommand, untouched.
    pub rest: Vec<String>,
}

/// Splits at the first token that is neither an option nor an option's value.
pub fn split(args: &[String]) -> Split {
    let mut out = Split::default();
    let mut i = 0;
    while i < args.len() {
        let token = &args[i];
        if !token.starts_with('-') || token == "-" {
            out.subcommand = Some(token.clone());
            out.rest = args[i + 1..].to_vec();
            return out;
        }
        out.globals.push(token.clone());
        let (name, attached) = option_name(token);
        match name {
            "-f" | "--file" => out.has_files = true,
            "-p" | "--project-name" => out.has_project = true,
            _ => {}
        }
        if VALUE_OPTIONS.contains(&name) && !attached && i + 1 < args.len() {
            out.globals.push(args[i + 1].clone());
            i += 1;
        }
        i += 1;
    }
    out
}

/// The values of every `-f`/`--file` in `globals`, in order.
pub fn file_values(globals: &[String]) -> Vec<String> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < globals.len() {
        let token = &globals[i];
        let (name, attached) = option_name(token);
        if matches!(name, "-f" | "--file") {
            if attached {
                let value = token
                    .strip_prefix("--file=")
                    .or_else(|| token.strip_prefix("-f"))
                    .unwrap_or_default();
                out.push(value.to_owned());
            } else if let Some(v) = globals.get(i + 1) {
                out.push(v.clone());
                i += 1;
            }
        } else if VALUE_OPTIONS.contains(&name) && !attached {
            i += 1;
        }
        i += 1;
    }
    out
}

/// The option's name and whether its value is attached (`--file=x`, `-fx`).
fn option_name(token: &str) -> (&str, bool) {
    if let Some(long) = token.strip_prefix("--") {
        return match long.split_once('=') {
            Some((name, _)) => (&token[..2 + name.len()], true),
            None => (token, false),
        };
    }
    // Short options with a value can be glued: `-fcompose.yaml`, `-pname`.
    if token.len() > 2 && (token.starts_with("-f") || token.starts_with("-p")) {
        return (&token[..2], true);
    }
    (token, false)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(args: &[&str]) -> Split {
        split(&args.iter().map(|a| (*a).to_owned()).collect::<Vec<_>>())
    }

    #[test]
    fn subcommand_and_rest() {
        let out = s(&["up", "-d", "--build", "web"]);
        assert_eq!(out.subcommand.as_deref(), Some("up"));
        assert_eq!(out.rest, ["-d", "--build", "web"]);
        assert!(out.globals.is_empty() && !out.has_files && !out.has_project);
    }

    #[test]
    fn file_option_forms() {
        for form in [
            &["-f", "x.yaml", "up"][..],
            &["--file", "x.yaml", "up"],
            &["--file=x.yaml", "up"],
            &["-fx.yaml", "up"],
        ] {
            let out = s(form);
            assert!(out.has_files, "{form:?}");
            assert_eq!(out.subcommand.as_deref(), Some("up"), "{form:?}");
            assert_eq!(out.globals, form[..form.len() - 1], "{form:?}");
        }
    }

    #[test]
    fn value_options_are_not_subcommands() {
        let out = s(&[
            "--profile",
            "debug",
            "-p",
            "custom",
            "--dry-run",
            "run",
            "web",
            "sh",
        ]);
        assert_eq!(
            out.globals,
            ["--profile", "debug", "-p", "custom", "--dry-run"]
        );
        assert!(out.has_project);
        assert_eq!(out.subcommand.as_deref(), Some("run"));
        assert_eq!(out.rest, ["web", "sh"]);
    }

    #[test]
    fn extracts_file_values() {
        let g: Vec<String> = [
            "-f",
            "a.yaml",
            "--profile",
            "x",
            "--file=b.yaml",
            "-fc.yaml",
            "--file",
            "d.yaml",
        ]
        .iter()
        .map(|a| (*a).to_owned())
        .collect();
        assert_eq!(file_values(&g), ["a.yaml", "b.yaml", "c.yaml", "d.yaml"]);
    }

    #[test]
    fn no_subcommand() {
        let out = s(&["--help"]);
        assert_eq!(out.subcommand, None);
        assert_eq!(out.globals, ["--help"]);
    }
}
