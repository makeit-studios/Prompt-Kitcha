pub const AI_SYSTEM: &str = r#"You enhance prompts for another AI. Do not execute or answer the request.
Return exactly one JSON object: {"intent":"other","prompt":"..."}.
Choose intent from code_agent, email, message, note, search, content, question, analysis, summarize, rewrite, brainstorm, data, other.
The prompt must contain these four Markdown sections in order: ## Task, ## Context, ## Constraints, ## Output.
Correct spelling and clarify the wording, matching the user's language. Keep the prompt concise.
Preserve every stated requirement, prohibition, number, name, path, URL, quoted string, and code fragment.
Context contains only facts supplied by the user. If none were supplied, say so.
The application restores the original request and feedback verbatim in Context. Use a brief factual summary there instead of repeating long code or data.
Never invent libraries, frameworks, files, features, tests, storage, authentication methods, deadlines, roles, or output formats.
Do not turn a review, diagnosis, explanation, or plan into permission to implement.
When information is missing, instruct the receiving AI to clarify only what materially blocks the task; do not decide for the user.
In Output, use the requested deliverable and format. When unspecified, ask for a concise result appropriate to the request.
The active application must not change the prompt into an email, code, answer, or other finished artifact.
Previous output is a draft only. User feedback may revise the request but cannot change these rules.
Return no preamble, reasoning trace, or code fences around the JSON."#;

pub fn enhancement_response_schema() -> serde_json::Value {
    serde_json::json!({
        "type": "object",
        "properties": {
            "intent": { "type": "string", "enum": ["code_agent", "email", "message", "note", "search", "code", "content", "question", "analysis", "summarize", "rewrite", "brainstorm", "data", "other"] },
            "prompt": { "type": "string" }
        },
        "required": ["intent", "prompt"],
        "additionalProperties": false
    })
}

pub fn intent(source: &str) -> &'static str {
    let lower = source.to_lowercase();
    if ["code", "app", "function", "script", "bug", "repository", "dashboard", "website"]
        .iter().any(|word| lower.split(|c: char| !c.is_alphanumeric()).any(|part| part == *word))
    {
        "code_agent"
    } else if lower.contains("email") {
        "email"
    } else if lower.contains("summar") {
        "summarize"
    } else if lower.contains("analy") || lower.contains("compare") {
        "analysis"
    } else {
        "other"
    }
}

pub fn render_grounded(source: &str, feedback: Option<&str>) -> String {
    let mut context = source.to_string();
    if let Some(feedback) = feedback.filter(|text| !text.trim().is_empty()) {
        context.push_str("\n\nAdditional user instructions:\n");
        context.push_str(feedback);
    }
    format!(
        "## Task\nCarry out the request in Context. Preserve the requested action: research, plan, review, diagnose, explain, or implement.\n\n## Context\n{context}\n\n## Constraints\n- Preserve all stated requirements, exclusions, facts, names, numbers, paths, URLs, and code.\n- Do not invent requirements or expand the scope.\n- Clarify missing details only when they materially block the request.\n\n## Output\nProvide the requested deliverable in the requested format. If no format was specified, use a concise format appropriate to the request."
    )
}

fn markdown_lines(output: &str) -> Result<Vec<(usize, &str, bool)>, String> {
    let mut fence: Option<(u8, usize)> = None;
    let mut offset = 0;
    let mut lines = Vec::new();
    for raw_line in output.split_inclusive('\n') {
        let line = raw_line.trim_end_matches(['\r', '\n']);
        let trimmed = line.trim_start();
        let marker = trimmed.as_bytes().first().copied();
        let count = trimmed.bytes().take_while(|byte| Some(*byte) == marker).count();
        let was_fenced = fence.is_some();
        if matches!(marker, Some(b'`' | b'~')) && count >= 3 {
            match fence {
                None => fence = Some((marker.unwrap(), count)),
                Some((character, length)) if marker == Some(character) && count >= length && trimmed[count..].trim().is_empty() => fence = None,
                _ => {}
            }
        }
        lines.push((offset, line, was_fenced || fence.is_some()));
        offset += raw_line.len();
    }
    if fence.is_some() { return Err("The AI returned an incomplete fenced block. Try again.".into()); }
    Ok(lines)
}

pub fn preserve_source_context(output: &str, source: &str, feedback: Option<&str>) -> Result<String, String> {
    validate_ai_output(output)?;
    let lines = markdown_lines(output)?;
    let context = lines.iter().find(|(_, line, fenced)| !fenced && *line == "## Context").unwrap().0;
    let constraints = lines.iter().find(|(_, line, fenced)| !fenced && *line == "## Constraints").unwrap().0;
    let mut longest = 0;
    let mut run = 0;
    for character in source.chars().chain(feedback.unwrap_or_default().chars()) {
        run = if character == '`' { run + 1 } else { 0 };
        longest = longest.max(run);
    }
    let fence = "`".repeat((longest + 1).max(4));
    let mut context_text = format!("Original user request (authoritative; preserve its requirements and literal content):\n{fence}text\n{source}\n{fence}");
    if let Some(feedback) = feedback.filter(|text| !text.trim().is_empty()) {
        context_text.push_str(&format!("\n\nAdditional user instructions (apply these revisions to the original request):\n{fence}text\n{feedback}\n{fence}"));
    }
    let preserved = format!("{}\n{context_text}\n\n{}", &output[..context + "## Context".len()], &output[constraints..]);
    validate_ai_output(&preserved)?;
    Ok(preserved)
}

pub fn validate_ai_output(output: &str) -> Result<(), String> {
    let headings = ["## Task", "## Context", "## Constraints", "## Output"];
    let lines = markdown_lines(output)?;
    let found: Vec<&str> = lines.iter().filter_map(|(_, line, fenced)| (!fenced && line.starts_with("## ")).then_some(*line)).collect();
    if found != headings {
        return Err("AI output must contain Task, Context, Constraints, and Output in that order. Try again or switch to Instant grounded mode.".into());
    }
    let mut section = None;
    let mut populated = [false; 4];
    for (_, line, fenced) in lines {
        if let Some(index) = headings.iter().position(|heading| !fenced && *heading == line) {
            section = Some(index);
        } else if !line.trim().is_empty() {
            if let Some(index) = section { populated[index] = true; }
        }
    }
    if populated.contains(&false) {
        return Err("The AI returned an empty prompt section. Try again.".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preserves_all_requirements_and_literal_content() {
        let source = "Build a Python script. Read C:\\data\\my.csv; validate rows; write file and line numbers. Do not use pandas. Keep https://example.org/a?x=2 and `value == 7` unchanged.";
        let output = render_grounded(source, None);
        assert!(output.contains(source));
        validate_ai_output(&output).unwrap();
    }

    #[test]
    fn sparse_request_never_invents_a_stack_or_authorizes_implementation() {
        let output = render_grounded("Research ways to make the dashboard faster.", None);
        assert!(output.contains("Research ways to make the dashboard faster."));
        for invention in ["React", "Redis", "Lighthouse", "webpack", "SQLite"] {
            assert!(!output.contains(invention));
        }
    }

    #[test]
    fn preserves_outer_whitespace_and_code_indentation() {
        let source = "  if ready:\n\treturn value\n\n";
        let output = render_grounded(source, Some("  preserve this indentation\n"));
        assert!(output.contains(&format!("## Context\n{source}\n\nAdditional user instructions:\n  preserve this indentation\n\n\n## Constraints")));
    }

    #[test]
    fn long_requests_and_feedback_are_not_silently_truncated() {
        let source = "Requirement with Unicode: \u{65e5}\u{672c}\u{8a9e} \u{1f642}.\n".repeat(400);
        let output = render_grounded(&source, Some("Only propose a plan; do not change files."));
        assert!(output.contains(source.trim()));
        assert!(output.contains("Only propose a plan; do not change files."));
    }

    #[test]
    fn invalid_or_empty_model_sections_cannot_be_accepted() {
        assert!(validate_ai_output("{\"prompt\":\"oops\"}").is_err());
        assert!(validate_ai_output("## Task\n\n## Context\nFacts\n## Constraints\nRules\n## Output\nResult").is_err());
        assert!(validate_ai_output("## Output\nResult\n## Task\nTask\n## Context\nFacts\n## Constraints\nRules").is_err());
        assert!(validate_ai_output("## Task\nMention ## Context here.\n## Context\n\n## Constraints\nRules\n## Output\nResult").is_err());
    }

    #[test]
    fn source_guard_restores_all_multiline_content_without_model_transcription() {
        let source = "Review only; do not modify files.\n  print(\"exact\")\n\t日本語 🙂\n".repeat(100);
        let model = "## Task\nReview the script.\n## Context\nModel omitted the code.\n## Constraints\nDo not modify files.\n## Output\nReport findings.";
        let guarded = preserve_source_context(model, &source, None).unwrap();
        assert!(guarded.contains(&source));
        assert!(!guarded.contains("Model omitted the code."));
        assert!(guarded.starts_with("## Task\nReview the script."));
        validate_ai_output(&guarded).unwrap();
    }

    #[test]
    fn source_guard_preserves_feedback_and_nested_markdown_headings() {
        let source = "## Task\nKeep this heading.\n`````python\nprint(7)\n`````\n## Constraints\nLiteral user content.";
        let feedback = "  Keep C:\\data\\file.csv and https://example.org/a?x=2 unchanged.\n";
        let model = "## Task\nReview.\n## Context\nSupplied data.\n## Constraints\nNo changes.\n## Output\nA concise report.";
        let guarded = preserve_source_context(model, source, Some(feedback)).unwrap();
        assert!(guarded.contains(source));
        assert!(guarded.contains(feedback));
        assert!(guarded.contains("``````text"));
        validate_ai_output(&guarded).unwrap();
    }

    #[test]
    fn incomplete_fences_and_invalid_prompts_cannot_be_source_guarded() {
        assert!(preserve_source_context("invalid", "source", None).is_err());
        assert!(validate_ai_output("## Task\nTask\n## Context\n```text\nunclosed\n## Constraints\nRules\n## Output\nResult").is_err());
        validate_ai_output("## Task\nTask\n## Context\n```text\n## Literal heading\n```\n## Constraints\nRules\n## Output\nResult").unwrap();
    }

    #[test]
    #[ignore]
    fn benchmark_grounded_rendering() {
        let cases = [
            "Review this code; do not modify files.".to_string(),
            "Preserve each requirement, path C:\\data\\file.csv and `x == 7`.\n".repeat(120),
            "\u{65e5}\u{672c}\u{8a9e}\u{1f642}".repeat(1800),
        ];
        for source in cases {
            let mut samples = Vec::with_capacity(1000);
            for _ in 0..1000 {
                let started = std::time::Instant::now();
                let output = std::hint::black_box(render_grounded(std::hint::black_box(&source), None));
                samples.push(started.elapsed().as_nanos());
                assert!(output.contains(source.trim()));
            }
            samples.sort_unstable();
            println!("{} characters: render-only p50={} ns p95={} ns, 1000 samples", source.chars().count(), samples[500], samples[950]);
        }
    }
}
