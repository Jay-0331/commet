//! Turn the staged diff + resolved `[style]` config into the system and
//! user prompts of a [`GenerateRequest`].
//!
//! Pure and I/O-free: the caller fetches the diff and resolves config,
//! this module assembles the two prompt strings. The system prompt
//! encodes the format rules (Conventional, gitmoji, …), length limits,
//! any few-shot examples, and a strict "output only the message"
//! instruction; the user prompt carries the file list and the fenced
//! diff.

use crate::config::schema::{MessageFormat, Style};
use crate::provider::GenerateRequest;

/// Per-run generation parameters, separate from the durable `[style]`
/// config. `extra_prompt` is appended to the system prompt under a
/// `USER OVERRIDE` header (the `-p` flag / `[style].extra_prompt`).
#[derive(Debug, Clone, Default)]
pub struct GenOpts {
    pub model: String,
    pub max_tokens: u32,
    pub temperature: f32,
    pub n: u8,
    pub extra_prompt: Option<String>,
    /// Recent accepted messages from this project (learning loop), shown
    /// to the model as a house-style reference. Empty = section omitted.
    pub examples: Vec<String>,
}

/// Assemble a [`GenerateRequest`] from style config, the diff, the list
/// of changed files, and the per-run options.
pub fn build(style: &Style, diff: &str, files: &[String], opts: &GenOpts) -> GenerateRequest {
    GenerateRequest {
        system_prompt: system_prompt(style, &opts.examples, opts.extra_prompt.as_deref()),
        user_prompt: user_prompt(diff, files),
        model: opts.model.clone(),
        max_tokens: opts.max_tokens,
        temperature: opts.temperature,
        n: opts.n,
    }
}

/// Build the system prompt: role, format rules, length limits, style
/// examples, recent-commit examples (`learned`), output contract, and
/// any user override.
pub fn system_prompt(style: &Style, learned: &[String], extra: Option<&str>) -> String {
    // Custom format hands the whole system prompt to the user (falling
    // back to the plain rules if they left it empty).
    if style.format == MessageFormat::Custom && !style.custom.system_prompt.trim().is_empty() {
        let mut s = style.custom.system_prompt.trim().to_string();
        append_learned(&mut s, learned);
        append_override(&mut s, extra);
        return s;
    }

    let mut s = String::from(
        "You are an expert software engineer writing a git commit message for the staged diff. \
         Write in the imperative mood, present tense.",
    );

    s.push_str("\n\n");
    s.push_str(&format_rules(style));

    s.push_str(&format!(
        "\n\nKeep the subject line at or under {} characters.",
        style.subject_max_len
    ));
    if style.include_body {
        s.push_str(&format!(
            " When a body adds useful context, add a blank line after the subject and wrap the \
             body at {} characters; omit the body for trivial changes.",
            style.body_wrap
        ));
    } else {
        s.push_str(" Output only the subject line — no body.");
    }

    if !style.examples.is_empty() {
        s.push_str("\n\nExamples of the desired style:");
        for example in &style.examples {
            s.push_str("\n- ");
            s.push_str(example);
        }
    }

    append_learned(&mut s, learned);

    s.push_str(
        "\n\nOutput ONLY the commit message text — no preamble, no explanation, no code fences, \
         no surrounding quotes.",
    );

    append_override(&mut s, extra);
    s
}

/// Append the recent-commits section, or nothing when the list is empty.
fn append_learned(s: &mut String, learned: &[String]) {
    if learned.is_empty() {
        return;
    }
    s.push_str("\n\n## Recent commits from this project\nMatch their voice and conventions:");
    for example in learned {
        s.push_str("\n- ");
        s.push_str(example);
    }
}

/// The format-specific rules block.
fn format_rules(style: &Style) -> String {
    match style.format {
        MessageFormat::Plain => {
            "Write a single concise sentence summarizing the change.".to_string()
        }
        MessageFormat::Conventional | MessageFormat::ConventionalBody => {
            let mut r = format!(
                "Use the Conventional Commits format: `type(scope): subject`. \
                 Allowed types: {}.",
                style.allowed_types.join(", ")
            );
            if !style.allowed_scopes.is_empty() {
                r.push_str(&format!(
                    " Prefer one of these scopes when applicable: {}.",
                    style.allowed_scopes.join(", ")
                ));
            } else {
                r.push_str(" Choose a short scope from the changed files, or omit it.");
            }
            if style.format == MessageFormat::ConventionalBody {
                r.push_str(" Always include a body explaining what changed and why.");
            }
            r
        }
        MessageFormat::Gitmoji => {
            "Start the subject with a single relevant gitmoji emoji, then a concise summary."
                .to_string()
        }
        MessageFormat::SubjectBody => {
            "Write a concise subject line, then a blank line, then a body explaining what \
             changed and why."
                .to_string()
        }
        // Custom with an empty custom prompt falls through to plain rules.
        MessageFormat::Custom => {
            "Write a single concise sentence summarizing the change.".to_string()
        }
    }
}

/// Append the user's extra instructions under a clearly-labelled header
/// so the model treats them as highest priority.
fn append_override(s: &mut String, extra: Option<&str>) {
    if let Some(extra) = extra {
        let extra = extra.trim();
        if !extra.is_empty() {
            s.push_str("\n\nUSER OVERRIDE (highest priority — follow exactly):\n");
            s.push_str(extra);
        }
    }
}

/// Build the user prompt: the changed-file list followed by the fenced
/// diff.
pub fn user_prompt(diff: &str, files: &[String]) -> String {
    let mut s = String::new();
    if !files.is_empty() {
        s.push_str("Files changed:\n");
        for f in files {
            s.push_str("- ");
            s.push_str(f);
            s.push('\n');
        }
        s.push('\n');
    }
    s.push_str("Staged diff:\n\n```diff\n");
    s.push_str(diff);
    if !diff.ends_with('\n') {
        s.push('\n');
    }
    s.push_str("```\n");
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::schema::Style;

    fn style(format: MessageFormat) -> Style {
        Style {
            format,
            ..Style::default()
        }
    }

    #[test]
    fn conventional_lists_allowed_types_and_length_limit() {
        let sp = system_prompt(&style(MessageFormat::Conventional), &[], None);
        assert!(sp.contains("Conventional Commits"));
        assert!(sp.contains("feat"));
        assert!(sp.contains("72 characters")); // default subject_max_len
        assert!(sp.contains("Output ONLY"));
    }

    #[test]
    fn conventional_body_forces_a_body() {
        let sp = system_prompt(&style(MessageFormat::ConventionalBody), &[], None);
        assert!(sp.contains("Always include a body"));
    }

    #[test]
    fn gitmoji_and_subject_body_have_their_rules() {
        assert!(system_prompt(&style(MessageFormat::Gitmoji), &[], None).contains("gitmoji"));
        assert!(
            system_prompt(&style(MessageFormat::SubjectBody), &[], None)
                .contains("blank line, then a body")
        );
    }

    #[test]
    fn custom_uses_the_configured_system_prompt() {
        let mut s = style(MessageFormat::Custom);
        s.custom.system_prompt = "MY CUSTOM RULES".into();
        let sp = system_prompt(&s, &[], None);
        assert!(sp.starts_with("MY CUSTOM RULES"));
        // The generic rules are not appended over a custom prompt.
        assert!(!sp.contains("Conventional Commits"));
    }

    #[test]
    fn empty_custom_prompt_falls_back_to_plain_rules() {
        let sp = system_prompt(&style(MessageFormat::Custom), &[], None);
        assert!(sp.contains("concise sentence"));
        assert!(sp.contains("Output ONLY"));
    }

    #[test]
    fn extra_prompt_appended_under_user_override() {
        let sp = system_prompt(
            &style(MessageFormat::Plain),
            &[],
            Some("use British spelling"),
        );
        assert!(sp.contains("USER OVERRIDE"));
        assert!(sp.contains("use British spelling"));
        // Empty / whitespace extra adds nothing.
        let sp2 = system_prompt(&style(MessageFormat::Plain), &[], Some("   "));
        assert!(!sp2.contains("USER OVERRIDE"));
    }

    #[test]
    fn examples_included_when_present_and_omitted_when_empty() {
        let with = system_prompt(&style(MessageFormat::Plain), &[], None);
        assert!(with.contains("Examples of the desired style"));
        assert!(with.contains("feat(auth): add OAuth device flow"));

        let mut bare = style(MessageFormat::Plain);
        bare.examples.clear();
        assert!(!system_prompt(&bare, &[], None).contains("Examples of the desired style"));
    }

    #[test]
    fn include_body_false_says_subject_only() {
        let mut s = style(MessageFormat::Plain);
        s.include_body = false;
        assert!(system_prompt(&s, &[], None).contains("only the subject line"));
    }

    #[test]
    fn user_prompt_lists_files_and_fences_the_diff() {
        let up = user_prompt(
            "diff --git a/x b/x\n+hello",
            &["src/x.rs".into(), "src/y.rs".into()],
        );
        assert!(up.contains("Files changed:"));
        assert!(up.contains("- src/x.rs"));
        assert!(up.contains("```diff\n"));
        assert!(up.contains("diff --git a/x b/x"));
        assert!(up.trim_end().ends_with("```"));
    }

    #[test]
    fn user_prompt_without_files_skips_the_list() {
        let up = user_prompt("some diff\n", &[]);
        assert!(!up.contains("Files changed:"));
        assert!(up.contains("```diff"));
    }

    #[test]
    fn build_assembles_generate_request() {
        let opts = GenOpts {
            model: "claude-sonnet-4-6".into(),
            max_tokens: 512,
            temperature: 0.3,
            n: 3,
            ..GenOpts::default()
        };
        let req = build(&style(MessageFormat::Conventional), "the diff", &[], &opts);
        assert_eq!(req.model, "claude-sonnet-4-6");
        assert_eq!(req.n, 3);
        assert_eq!(req.max_tokens, 512);
        assert!(req.system_prompt.contains("Conventional Commits"));
        assert!(req.user_prompt.contains("the diff"));
    }

    #[test]
    fn learned_examples_add_a_recent_commits_section_in_order() {
        let learned = vec!["feat: newest".to_string(), "fix: older".to_string()];
        let sp = system_prompt(&style(MessageFormat::Plain), &learned, None);
        assert!(sp.contains("## Recent commits from this project"));
        // Order preserved.
        let newest = sp.find("feat: newest").unwrap();
        let older = sp.find("fix: older").unwrap();
        assert!(newest < older);
        // Section sits before the output contract and any override.
        assert!(sp.find("## Recent commits").unwrap() < sp.find("Output ONLY").unwrap());
    }

    #[test]
    fn no_recent_commits_section_when_empty() {
        let sp = system_prompt(&style(MessageFormat::Plain), &[], None);
        assert!(!sp.contains("Recent commits from this project"));
    }

    #[test]
    fn build_threads_examples_into_the_prompt() {
        let opts = GenOpts {
            examples: vec!["chore: bump deps".into()],
            ..GenOpts::default()
        };
        let req = build(&style(MessageFormat::Plain), "d", &[], &opts);
        assert!(req.system_prompt.contains("chore: bump deps"));
        assert!(
            req.system_prompt
                .contains("Recent commits from this project")
        );
    }
}
