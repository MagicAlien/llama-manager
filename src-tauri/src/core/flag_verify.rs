//! T-023 — flag verification, health endpoint, and registration channel.
//!
//! `llama-server.exe --help` is the only authority for which flags a build
//! accepts and what value type each takes (`AGENTS.md` §1, `docs/LLAMACPP.md`).
//! This module parses a captured `--help` into `Vec<VerifiedFlag>`, records
//! the build's health endpoint, settles `PLAN.md` §2.1's first question as
//! `RuntimeBuild.registration_channel`, and checks every flag named in
//! `docs/LLAMACPP.md` against the verified list.
//!
//! **The parser is format-tolerant, not format-agnostic.** It is built on the
//! invariants measured across four real captures (b5559, b7213, b9196,
//! b10809, spanning ~15 months): a flag line starts with `-` at column 0, and
//! the description — when present on the same line — always begins at a fixed
//! column (40 in every capture). A value placeholder may contain spaces
//! (`FNAME SCALE`, `TARGET DRAFT`), so a gap-based heuristic is unsafe; the
//! fixed column is the only reliable split. When the name field overruns that
//! column, the description wraps to the following line(s).
//!
//! **Degradation, never a false positive.** An input that yields no flag lines
//! at all is an unrecognized format: the parser returns an empty verified list
//! plus a warning. It never panics and never invents a flag it did not see.

use std::fmt::Write as _;

use crate::core::types::{RegistrationChannel, VerifiedFlag};

/// The fixed column at which a same-line description begins, measured across
/// the four committed captures. See the module doc for why a gap heuristic is
/// rejected in favour of this.
const DESC_COL: usize = 40;

/// A parsed `--help` capture.
#[derive(Debug, Clone, PartialEq)]
pub struct ParsedHelp {
    /// Every flag the capture names, in the order it appears. A flag that
    /// appears under several spellings (`-fa, --flash-attn`) is recorded once,
    /// keyed by its long form.
    pub flags: Vec<VerifiedFlag>,
    /// `false` when the input was not recognized as a llama.cpp `--help`
    /// layout at all — `flags` is then empty and `warnings` explains why.
    pub recognized: bool,
    /// Human-readable notes (unrecognized format, a line that could not be
    /// split, …). Empty when the capture parsed cleanly.
    pub warnings: Vec<String>,
}

/// One flag as named in `docs/LLAMACPP.md`, with the value type that document
/// claims. The check in [`check_doc_flags`] compares each of these against the
/// verified list and names any that are missing or retyped.
///
/// `name` is the long form (`--flash-attn`); `alias` is the short form when the
/// document lists one (`-fa`). A flag is "present" if either spelling is in the
/// verified list.
#[derive(Debug, Clone, Copy)]
pub struct DocFlag {
    pub name: &'static str,
    pub alias: Option<&'static str>,
    /// The value type `docs/LLAMACPP.md` claims. `None` means "the document
    /// does not model a type" (e.g. a flag listed for completeness only), in
    /// which case only presence is checked, not the type.
    pub expected: Option<ExpectedType>,
}

/// A value type as claimed by `docs/LLAMACPP.md`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExpectedType {
    /// No value: the flag's presence is the whole argument (`--no-webui`).
    Boolean,
    /// Takes a single value of an unenumerated type (`--port N`, `--host ADDR`).
    Value,
    /// Takes a value from a fixed set (`--flash-attn` as `on|off|auto`).
    Enumerated,
}

/// The outcome of checking `docs/LLAMACPP.md` against a verified list.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct DocFlagReport {
    /// Flags the document names but the build does not.
    pub missing: Vec<String>,
    /// Flags the build has under a different value type than documented. Each
    /// entry names the flag and what the document expected versus what the
    /// build shows — this is the "retyped" case `AGENTS.md` §1 warns about.
    pub retyped: Vec<String>,
    /// Flags the document names and the build confirms, type and all.
    pub confirmed: Vec<String>,
}

impl DocFlagReport {
    /// `true` when every documented flag is present and correctly typed.
    pub fn clean(&self) -> bool {
        self.missing.is_empty() && self.retyped.is_empty()
    }
}

/// Parse a `llama-server.exe --help` capture.
///
/// Returns a `ParsedHelp`. An unrecognized layout (no flag lines at all) yields
/// `recognized: false`, an empty `flags` list, and a warning — never a panic.
pub fn parse_help(help: &str) -> ParsedHelp {
    let mut flags: Vec<VerifiedFlag> = Vec::new();
    let mut warnings: Vec<String> = Vec::new();

    for raw in help.lines() {
        let line = raw.trim_end();
        // A flag line starts with `-` at column 0. Section headers are runs of
        // dashes (`----- Server options -----`); a single leading `-` is the
        // only thing that starts a flag.
        if !line.starts_with('-') || line.starts_with("-----") {
            continue;
        }

        // Split the name field from the description at the fixed column.
        let (name_field, _desc) = split_name_desc(line);
        let Some(parsed) = parse_name_field(&name_field) else {
            // A `-`-led line whose name field holds no flag token: not a flag
            // in the layout we understand. Note it and move on — a warning,
            // never a false positive.
            warnings.push(format!("unrecognized flag line: {line}"));
            continue;
        };

        // Record each flag the line names, keyed by its long form. A line can
        // name several (`-fa, --flash-attn` or `--jinja, --no-jinja`); each is
        // a distinct flag sharing the line's value semantics.
        for nf in parsed {
            if let Some(existing) = flags.iter_mut().find(|f| f.name == nf.name) {
                if existing.allowed_values.is_none() && nf.allowed_values.is_some() {
                    existing.allowed_values = nf.allowed_values;
                }
                if !existing.takes_value {
                    existing.takes_value = nf.takes_value;
                }
            } else {
                flags.push(VerifiedFlag {
                    name: nf.name,
                    takes_value: nf.takes_value,
                    allowed_values: nf.allowed_values,
                });
            }
        }
    }

    if flags.is_empty() {
        // No flag lines at all: the input is not the layout we recognize.
        warnings.push(
            "no flag lines recognized; the capture does not match the expected \
             `llama-server --help` layout — treating the build as unverified"
                .to_string(),
        );
        return ParsedHelp {
            flags: Vec::new(),
            recognized: false,
            warnings,
        };
    }

    ParsedHelp {
        flags,
        recognized: true,
        warnings,
    }
}

/// Split a flag line into its name field and (same-line) description.
///
/// Returns `(name_field, has_same_line_description)`. When the name field
/// overruns the description column, the description is on a following line and
/// `has_same_line_description` is `false`.
fn split_name_desc(line: &str) -> (String, bool) {
    // The description, if on this line, begins exactly at DESC_COL. For that to
    // be a clean split, the character just before it (column DESC_COL-1) must be
    // a space and the character at DESC_COL must be the first description
    // character (non-space). Otherwise the name field runs past the column and
    // the description wraps.
    let chars: Vec<char> = line.chars().collect();
    if chars.len() > DESC_COL && chars[DESC_COL - 1] == ' ' && chars[DESC_COL] != ' ' {
        let cut: String = chars.iter().take(DESC_COL).collect();
        (cut.trim_end().to_string(), true)
    } else {
        (line.to_string(), false)
    }
}

/// A single flag line's name field, parsed into one `NameField` per flag name
/// it names. A line can name several flags: a short/long alias pair
/// (`-fa, --flash-attn`) or a boolean pair (`--jinja, --no-jinja`). Each
/// `--` token is a distinct flag and is recorded under its own name, sharing
/// the line's value semantics; a lone short form is used only if no long form
/// is present.
struct NameField {
    /// The canonical name, e.g. `--flash-attn`.
    name: String,
    takes_value: bool,
    allowed_values: Option<Vec<String>>,
}

/// Parse a name field into the flags it names. Returns `None` when the field
/// holds no flag token (so the caller can flag the line as unrecognized).
fn parse_name_field(field: &str) -> Option<Vec<NameField>> {
    let tokens: Vec<&str> = field.split_whitespace().collect();
    let mut names: Vec<String> = Vec::new();
    let mut i = 0;
    // Consume the flag tokens: every token that starts with `-` is a spelling.
    // Every `--` token is a distinct flag (a boolean pair names two); a lone
    // short form is kept only if no long form appears.
    while i < tokens.len() && tokens[i].starts_with('-') {
        let tok = tokens[i].trim_end_matches(',');
        // Every `--` token is a distinct flag; a lone short form is kept only
        // when no long form appeared earlier on the line.
        if tok.starts_with("--") || names.is_empty() {
            names.push(tok.to_string());
        }
        i += 1;
    }
    if names.is_empty() {
        return None;
    }

    // The first remaining token, if any, is the value placeholder. Its
    // semantics apply to every flag name on the line.
    let placeholder = tokens.get(i).copied();
    let (takes_value, allowed_values) = match placeholder {
        None => (false, None),
        Some(ph) => match parse_placeholder(ph) {
            Placeholder::Enumerated(values) => (true, Some(values)),
            Placeholder::Value => (true, None),
        },
    };

    Some(
        names
            .into_iter()
            .map(|name| NameField {
                name,
                takes_value,
                allowed_values: allowed_values.clone(),
            })
            .collect(),
    )
}

/// The two shapes a value placeholder can take.
enum Placeholder {
    /// `{a,b}` / `[a|b]` / `<a|b>`: a fixed set of accepted values.
    Enumerated(Vec<String>),
    /// A bare name (`N`, `PATH`, `TYPE`): takes a value, not enumerated.
    Value,
}

fn parse_placeholder(ph: &str) -> Placeholder {
    let inner = ph
        .strip_prefix('{')
        .and_then(|s| s.strip_suffix('}'))
        .or_else(|| ph.strip_prefix('[').and_then(|s| s.strip_suffix(']')))
        .or_else(|| ph.strip_prefix('<').and_then(|s| s.strip_suffix('>')));
    match inner {
        Some(inner) => {
            let values: Vec<String> = inner
                .split(['|', ','])
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string)
                .collect();
            if values.is_empty() {
                Placeholder::Value
            } else {
                Placeholder::Enumerated(values)
            }
        }
        None => Placeholder::Value,
    }
}

/// Settle `PLAN.md` §2.1's first question from the help text: can a
/// `--models-preset` entry declare a model by absolute path?
///
/// The rule is deliberately narrow and evidence-bound. The help text answers
/// the question only if it says `--models-preset` takes a **path** to a preset
/// file *and* describes the presets as introducing models (rather than only
/// configuring ones a scan already found). Absent that, the honest answer is
/// `Undetermined` — the text does not say, so we do not infer.
///
/// In practice every capture to date yields `Undetermined`: the help line reads
/// "path to INI file containing model presets for the router server", which
/// names the file but never states whether an entry can *introduce* a model by
/// path. That is exactly the question T-025 answers by observation.
pub fn registration_channel_from_help(help: &str) -> RegistrationChannel {
    // Find the `--models-preset` line (and any same-line description).
    let preset_line = help.lines().find(|l| l.contains("--models-preset"));
    let Some(line) = preset_line else {
        // No `--models-preset` at all: the build has no preset channel to
        // declare a path through. The question is unanswerable from this text.
        return RegistrationChannel::Undetermined;
    };

    let lower = line.to_lowercase();
    // The text must both name a path and describe presets as introducing models.
    // "path to INI file containing model presets" names the file but does not
    // say an entry can declare a path — so require the stronger claim.
    let names_path = lower.contains("path");
    let introduces_model = lower.contains("declare")
        || lower.contains("introduce")
        || (lower.contains("by path") && lower.contains("model"));
    if names_path && introduces_model {
        RegistrationChannel::PresetDeclaresPath
    } else {
        RegistrationChannel::Undetermined
    }
}

/// The health endpoint a build offers, from its help text.
///
/// T-040's `Starting` transition depends on this and must not guess. The rule:
/// `/health` where the build names it, otherwise `/props`. Every capture to
/// date names only `POST /props` (and never `/health`), so all of them record
/// `/props`.
pub fn health_endpoint_from_help(help: &str) -> &'static str {
    if help.contains("/health") {
        "/health"
    } else {
        "/props"
    }
}

/// Check every flag named in `docs/LLAMACPP.md` against a verified list.
///
/// `doc_flags` is the document's claims (see [`DOC_FLAGS`]); `verified` is what
/// the build actually accepts. A flag the document names but the build lacks is
/// `missing`; a flag the build has under a different value type is `retyped`.
/// Both cases name the specific flag, per the task's acceptance criteria.
pub fn check_doc_flags(doc_flags: &[DocFlag], verified: &[VerifiedFlag]) -> DocFlagReport {
    let mut report = DocFlagReport::default();
    for df in doc_flags {
        // Present if either the long form or the short alias is verified.
        let found = verified
            .iter()
            .find(|f| f.name == df.name || (df.alias.is_some_and(|a| f.name == a)));
        match found {
            None => report.missing.push(df.name.to_string()),
            Some(flag) => {
                match df.expected {
                    Some(expected) => {
                        if !types_match(expected, flag) {
                            report.retyped.push(format!(
                                "{}: document expects {}, build shows {}",
                                df.name,
                                describe_expected(expected),
                                describe_verified(flag)
                            ));
                        } else {
                            report.confirmed.push(df.name.to_string());
                        }
                    }
                    // The document does not model a type: presence is enough.
                    None => report.confirmed.push(df.name.to_string()),
                }
            }
        }
    }
    report
}

/// The flags named in `docs/LLAMACPP.md` (its §1–§4 tables), with the value
/// type each claims. This is the document's claim set, not the verified list —
/// [`check_doc_flags`] is what turns the claims into evidence.
///
/// Types are transcribed from the document's own table (its "Type" column),
/// not from what the build actually shows: the check's job is to surface
/// where the two disagree. `expected: None` marks a flag the document lists
/// for completeness without modelling a value type (no current entry uses it;
/// kept for flags the doc names but does not type).
pub const DOC_FLAGS: &[DocFlag] = &[
    // §1 Router mode
    DocFlag {
        name: "--models-dir",
        alias: None,
        expected: Some(ExpectedType::Value),
    },
    DocFlag {
        name: "--models-preset",
        alias: None,
        expected: Some(ExpectedType::Value),
    },
    DocFlag {
        name: "--models-max",
        alias: None,
        expected: Some(ExpectedType::Value),
    },
    DocFlag {
        name: "--models-autoload",
        alias: None,
        expected: Some(ExpectedType::Boolean),
    },
    DocFlag {
        name: "--no-models-autoload",
        alias: None,
        expected: Some(ExpectedType::Boolean),
    },
    // §2 Server and binding
    DocFlag {
        name: "--host",
        alias: None,
        expected: Some(ExpectedType::Value),
    },
    DocFlag {
        name: "--port",
        alias: None,
        expected: Some(ExpectedType::Value),
    },
    DocFlag {
        name: "--no-webui",
        alias: None,
        expected: Some(ExpectedType::Boolean),
    },
    DocFlag {
        name: "--api-key",
        alias: None,
        expected: Some(ExpectedType::Value), // doc §2: "string" (the app never emits it)
    },
    DocFlag {
        name: "--jinja",
        alias: None,
        expected: Some(ExpectedType::Boolean),
    },
    // §3 Per-model launch flags
    DocFlag {
        name: "--n-gpu-layers",
        alias: Some("-ngl"),
        expected: Some(ExpectedType::Value),
    },
    DocFlag {
        name: "--ctx-size",
        alias: Some("-c"),
        expected: Some(ExpectedType::Value),
    },
    DocFlag {
        name: "--batch-size",
        alias: Some("-b"),
        expected: Some(ExpectedType::Value),
    },
    DocFlag {
        name: "--ubatch-size",
        alias: Some("-ub"),
        expected: Some(ExpectedType::Value),
    },
    // The standing example of a retyped flag: tri-state in recent builds,
    // boolean in older ones.
    DocFlag {
        name: "--flash-attn",
        alias: Some("-fa"),
        expected: Some(ExpectedType::Enumerated),
    },
    // The document's §3 table types these as "enum". The build's `--help`
    // shows a bare `TYPE` placeholder (a value, not an enumerated set), so
    // `check_doc_flags` reports them as retyped — a real doc/build divergence,
    // surfaced rather than papered over.
    DocFlag {
        name: "--cache-type-k",
        alias: Some("-ctk"),
        expected: Some(ExpectedType::Enumerated), // doc §3: "enum"
    },
    DocFlag {
        name: "--cache-type-v",
        alias: Some("-ctv"),
        expected: Some(ExpectedType::Enumerated), // doc §3: "enum"
    },
    DocFlag {
        name: "--n-cpu-moe",
        alias: None,
        expected: Some(ExpectedType::Value),
    },
    DocFlag {
        name: "--tensor-split",
        alias: None,
        expected: Some(ExpectedType::Value),
    },
    DocFlag {
        name: "--main-gpu",
        alias: Some("-mg"),
        expected: Some(ExpectedType::Value),
    },
    DocFlag {
        name: "--split-mode",
        alias: Some("-sm"),
        expected: Some(ExpectedType::Enumerated),
    },
    DocFlag {
        name: "--no-mmap",
        alias: None,
        expected: Some(ExpectedType::Boolean),
    },
    DocFlag {
        name: "--mlock",
        alias: None,
        expected: Some(ExpectedType::Boolean),
    },
    DocFlag {
        name: "--threads",
        alias: Some("-t"),
        expected: Some(ExpectedType::Value),
    },
    DocFlag {
        name: "--chat-template",
        alias: None,
        expected: Some(ExpectedType::Value),
    },
    DocFlag {
        name: "--mmproj",
        alias: None,
        expected: Some(ExpectedType::Value),
    },
    // §4 Sampling defaults (all [assumed], value-taking)
    DocFlag {
        name: "--temp",
        alias: None,
        expected: Some(ExpectedType::Value),
    },
    DocFlag {
        name: "--top-p",
        alias: None,
        expected: Some(ExpectedType::Value),
    },
    DocFlag {
        name: "--top-k",
        alias: None,
        expected: Some(ExpectedType::Value),
    },
    DocFlag {
        name: "--min-p",
        alias: None,
        expected: Some(ExpectedType::Value),
    },
    DocFlag {
        name: "--repeat-penalty",
        alias: None,
        expected: Some(ExpectedType::Value),
    },
    DocFlag {
        name: "--presence-penalty",
        alias: None,
        expected: Some(ExpectedType::Value),
    },
    DocFlag {
        name: "--frequency-penalty",
        alias: None,
        expected: Some(ExpectedType::Value),
    },
    DocFlag {
        name: "--seed",
        alias: None,
        expected: Some(ExpectedType::Value),
    },
];

fn types_match(expected: ExpectedType, flag: &VerifiedFlag) -> bool {
    match expected {
        ExpectedType::Boolean => !flag.takes_value,
        ExpectedType::Value => flag.takes_value && flag.allowed_values.is_none(),
        ExpectedType::Enumerated => flag.takes_value && flag.allowed_values.is_some(),
    }
}

fn describe_expected(expected: ExpectedType) -> &'static str {
    match expected {
        ExpectedType::Boolean => "a boolean flag",
        ExpectedType::Value => "a value-taking flag",
        ExpectedType::Enumerated => "an enumerated-value flag",
    }
}

fn describe_verified(flag: &VerifiedFlag) -> String {
    match (&flag.takes_value, &flag.allowed_values) {
        (false, _) => "a boolean flag".to_string(),
        (true, None) => "a value-taking flag".to_string(),
        (true, Some(values)) => format!("an enumerated-value flag [{}]", values.join("|")),
    }
}

/// Render `docs/verified-flags.md` for a single build.
///
/// The output is a pure function of its arguments: for a given build tag, flag
/// list, health endpoint, and registration channel it is byte-for-byte stable,
/// which is what lets the export script's snapshot assertion hold. Flags are
/// sorted by name so the ordering never depends on capture order.
pub fn render_verified_flags_md(
    build_tag: &str,
    flags: &[VerifiedFlag],
    health_endpoint: &str,
    channel: &RegistrationChannel,
) -> String {
    let mut sorted: Vec<&VerifiedFlag> = flags.iter().collect();
    sorted.sort_by(|a, b| a.name.cmp(&b.name));

    let channel_str = match channel {
        RegistrationChannel::PresetDeclaresPath => "PresetDeclaresPath",
        RegistrationChannel::ScanOnly => "ScanOnly",
        RegistrationChannel::Undetermined => "Undetermined",
    };

    let mut out = String::new();
    let _ = writeln!(out, "# Verified flags — {build_tag}");
    let _ = writeln!(
        out,
        "Generated by `scripts/export-verified-flags.ps1` from the database; do not edit by hand."
    );
    let _ = writeln!(out);
    let _ = writeln!(out, "Build: `{build_tag}`");
    let _ = writeln!(out, "Health endpoint: `{health_endpoint}`");
    let _ = writeln!(out, "Registration channel: `{channel_str}`");
    let _ = writeln!(out);
    let _ = writeln!(
        out,
        "Every line below was read from a real `llama-server.exe --help` capture."
    );
    let _ = writeln!(
        out,
        "A flag absent here, or of a different type, is a discrepancy (`AGENTS.md` §1)."
    );
    let _ = writeln!(out);
    let _ = writeln!(out, "| Flag | Takes value | Allowed values |");
    let _ = writeln!(out, "|---|---|---|");
    for f in sorted {
        let takes = if f.takes_value { "yes" } else { "no" };
        let allowed = match &f.allowed_values {
            None => "—".to_string(),
            Some(values) => format!("[{}]", values.join("|")),
        };
        let _ = writeln!(out, "| `{}` | {} | {} |", f.name, takes, allowed);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::Path;

    /// The four committed real captures. Their path is relative to the crate
    /// root (`src-tauri/`), which is `CARGO_MANIFEST_DIR`.
    fn capture(tag: &str) -> String {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("fixtures")
            .join("help")
            .join(format!("{tag}-help.txt"));
        fs::read_to_string(&path).unwrap_or_else(|e| panic!("missing fixture {path:?}: {e}"))
    }

    fn flag<'a>(flags: &'a [VerifiedFlag], name: &str) -> &'a VerifiedFlag {
        flags
            .iter()
            .find(|f| f.name == name)
            .unwrap_or_else(|| panic!("flag {name} not found"))
    }

    // ── Parser against the real captures ──────────────────────────────

    #[test]
    fn b10809_parses_and_classifies() {
        let parsed = parse_help(&capture("b10809"));
        assert!(parsed.recognized, "b10809 must be recognized");
        assert!(
            parsed.flags.len() > 300,
            "b10809 should have hundreds of flags"
        );

        // Boolean flag.
        let no_webui = flag(&parsed.flags, "--no-webui");
        assert!(!no_webui.takes_value);
        assert!(no_webui.allowed_values.is_none());

        // Value flag.
        let port = flag(&parsed.flags, "--port");
        assert!(port.takes_value);
        assert!(port.allowed_values.is_none());

        // Enumerated flag with the exact value set.
        let flash = flag(&parsed.flags, "--flash-attn");
        assert!(flash.takes_value);
        assert_eq!(
            flash.allowed_values,
            Some(vec![
                "on".to_string(),
                "off".to_string(),
                "auto".to_string()
            ])
        );

        // A boolean pair: both spellings present, both boolean.
        assert!(!flag(&parsed.flags, "--models-autoload").takes_value);
        assert!(!flag(&parsed.flags, "--no-models-autoload").takes_value);

        // Router flags exist in b10809.
        assert!(flag(&parsed.flags, "--models-dir").takes_value);
        assert!(flag(&parsed.flags, "--models-preset").takes_value);
    }

    #[test]
    fn b7213_parses_and_classifies() {
        let parsed = parse_help(&capture("b7213"));
        assert!(parsed.recognized);
        assert!(parsed.flags.len() > 200);

        // b7213 has no router flags.
        assert!(parsed.flags.iter().all(|f| f.name != "--models-preset"));
        assert!(parsed.flags.iter().all(|f| f.name != "--models-dir"));

        // --flash-attn is enumerated in b7213 (the retyped case vs b5559).
        let flash = flag(&parsed.flags, "--flash-attn");
        assert!(flash.takes_value);
        assert_eq!(
            flash.allowed_values,
            Some(vec![
                "on".to_string(),
                "off".to_string(),
                "auto".to_string()
            ])
        );

        // --split-mode enumerates three values in b7213.
        let split = flag(&parsed.flags, "--split-mode");
        assert_eq!(
            split.allowed_values,
            Some(vec![
                "none".to_string(),
                "layer".to_string(),
                "row".to_string()
            ])
        );
    }

    #[test]
    fn b5559_parses_and_flash_attn_is_boolean() {
        let parsed = parse_help(&capture("b5559"));
        assert!(parsed.recognized);
        assert!(parsed.flags.len() > 150);

        // The retyped-flag case: --flash-attn is a plain boolean in b5559.
        let flash = flag(&parsed.flags, "--flash-attn");
        assert!(!flash.takes_value, "--flash-attn must be boolean in b5559");
        assert!(flash.allowed_values.is_none());

        // --n-cpu-moe is absent in b5559.
        assert!(parsed.flags.iter().all(|f| f.name != "--n-cpu-moe"));

        // --split-mode enumerates three values in b5559.
        let split = flag(&parsed.flags, "--split-mode");
        assert_eq!(
            split.allowed_values,
            Some(vec![
                "none".to_string(),
                "layer".to_string(),
                "row".to_string()
            ])
        );
    }

    #[test]
    fn b9196_parses_and_classifies() {
        let parsed = parse_help(&capture("b9196"));
        assert!(parsed.recognized);
        assert!(parsed.flags.len() > 300);

        // b9196 has router flags and an enumerated --flash-attn.
        assert!(flag(&parsed.flags, "--models-preset").takes_value);
        let flash = flag(&parsed.flags, "--flash-attn");
        assert_eq!(
            flash.allowed_values,
            Some(vec![
                "on".to_string(),
                "off".to_string(),
                "auto".to_string()
            ])
        );
        // --split-mode enumerates four values in b9196.
        let split = flag(&parsed.flags, "--split-mode");
        assert_eq!(
            split.allowed_values,
            Some(vec![
                "none".to_string(),
                "layer".to_string(),
                "row".to_string(),
                "tensor".to_string()
            ])
        );
    }

    // ── Degradation: never panic, never a false positive ──────────────

    #[test]
    fn unrecognized_format_yields_empty_list_and_warning() {
        let parsed = parse_help("this is not a llama.cpp help output\njust some prose\n");
        assert!(!parsed.recognized);
        assert!(parsed.flags.is_empty(), "no false positives allowed");
        assert!(!parsed.warnings.is_empty());
    }

    #[test]
    fn empty_input_yields_empty_list() {
        let parsed = parse_help("");
        assert!(!parsed.recognized);
        assert!(parsed.flags.is_empty());
    }

    #[test]
    fn section_headers_are_not_flags() {
        let parsed = parse_help("----- Server options -----\n----- Model options -----\n");
        assert!(parsed.flags.is_empty());
        assert!(!parsed.recognized);
    }

    // ── registration channel and health endpoint ──────────────────────

    #[test]
    fn registration_channel_is_undetermined_for_all_captures() {
        // None of the four captures answers whether a preset entry can declare
        // a model by absolute path, so all must be Undetermined — not inferred.
        for tag in ["b10809", "b7213", "b5559", "b9196"] {
            let channel = registration_channel_from_help(&capture(tag));
            assert_eq!(
                channel,
                RegistrationChannel::Undetermined,
                "{tag} must be Undetermined"
            );
        }
    }

    #[test]
    fn health_endpoint_is_props_for_all_captures() {
        // None of the four captures names /health; all name POST /props.
        for tag in ["b10809", "b7213", "b5559", "b9196"] {
            assert_eq!(
                health_endpoint_from_help(&capture(tag)),
                "/props",
                "{tag} must be /props"
            );
        }
    }

    // ── doc-flag check: missing and retyped are named ─────────────────

    #[test]
    fn doc_check_names_missing_and_retyped_flags() {
        // b5559: --flash-attn is boolean (document expects enumerated) →
        // retyped; --n-cpu-moe and the router flags are absent → missing.
        let parsed = parse_help(&capture("b5559"));
        let report = check_doc_flags(DOC_FLAGS, &parsed.flags);
        assert!(
            report.retyped.iter().any(|s| s.starts_with("--flash-attn")),
            "b5559 --flash-attn must be retyped, got {:?}",
            report.retyped
        );
        assert!(
            report.missing.iter().any(|s| s == "--n-cpu-moe"),
            "b5559 --n-cpu-moe must be missing, got {:?}",
            report.missing
        );
        assert!(
            report.missing.iter().any(|s| s == "--models-preset"),
            "b5559 --models-preset must be missing, got {:?}",
            report.missing
        );
        // A flag present and correctly typed is confirmed.
        assert!(report.confirmed.iter().any(|s| s == "--port"));
    }

    #[test]
    fn doc_check_on_b10809_confirms_flash_attn() {
        let parsed = parse_help(&capture("b10809"));
        let report = check_doc_flags(DOC_FLAGS, &parsed.flags);
        // b10809 has --flash-attn enumerated, so it is confirmed, not retyped.
        assert!(
            report.confirmed.iter().any(|s| s == "--flash-attn"),
            "b10809 --flash-attn must be confirmed, retyped={:?}",
            report.retyped
        );
        assert!(!report.retyped.iter().any(|s| s.starts_with("--flash-attn")));
        // b10809 has all the router flags, so none of them is missing.
        assert!(!report.missing.iter().any(|s| s == "--models-preset"));
        assert!(!report.missing.iter().any(|s| s == "--models-dir"));
    }

    // ── markdown rendering: byte-stable ───────────────────────────────

    #[test]
    fn render_is_byte_stable_and_sorted() {
        let flags = vec![
            VerifiedFlag {
                name: "--zeta".to_string(),
                takes_value: false,
                allowed_values: None,
            },
            VerifiedFlag {
                name: "--alpha".to_string(),
                takes_value: true,
                allowed_values: Some(vec!["on".to_string(), "off".to_string()]),
            },
            VerifiedFlag {
                name: "--mid".to_string(),
                takes_value: true,
                allowed_values: None,
            },
        ];
        let a = render_verified_flags_md(
            "b9196",
            &flags,
            "/props",
            &RegistrationChannel::Undetermined,
        );
        // A second render with the same input (flags in a different order) is
        // byte-for-byte identical: sorting removes the capture-order dependency.
        let shuffled = vec![flags[2].clone(), flags[0].clone(), flags[1].clone()];
        let b = render_verified_flags_md(
            "b9196",
            &shuffled,
            "/props",
            &RegistrationChannel::Undetermined,
        );
        assert_eq!(a, b, "render must be byte-stable for the same build state");
        // Sorted: --alpha before --mid before --zeta.
        let alpha = a.find("--alpha").unwrap();
        let mid = a.find("--mid").unwrap();
        let zeta = a.find("--zeta").unwrap();
        assert!(alpha < mid && mid < zeta);
        // Header carries the build, endpoint, and channel.
        assert!(a.contains("Build: `b9196`"));
        assert!(a.contains("Health endpoint: `/props`"));
        assert!(a.contains("Registration channel: `Undetermined`"));
    }
}
