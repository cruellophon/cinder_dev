use harness_store::{AgentMode, SkillDefinition};
use std::path::Path;

pub fn builtin_skills() -> Vec<SkillDefinition> {
    vec![
        SkillDefinition {
            slug: "plan".into(),
            name: "Plan First".into(),
            description: "Inspect workspace read-only and submit a structured Plan Artifact for review.".into(),
            prompt_template: "Investigate the workspace read-only (use list_dir, read_file, search_knowledge, or delegate_research) and submit a clear implementation plan via `submit_plan` before editing any files.\n\nGoal: {input}".into(),
            recommended_mode: AgentMode::Plan,
            is_builtin: true,
        },
        SkillDefinition {
            slug: "grill-me".into(),
            name: "Grill Me (Interview)".into(),
            description: "Ask targeted clarifying questions via `ask_user` to lock down design choices.".into(),
            prompt_template: "Before proposing edits, inspect relevant files and interview me using `ask_user` with concise multiple-choice options to resolve ambiguity and edge cases.\n\nTopic: {input}".into(),
            recommended_mode: AgentMode::Plan,
            is_builtin: true,
        },
        SkillDefinition {
            slug: "review".into(),
            name: "Code & Uni Review".into(),
            description: "Audit workspace code, scripts, or academic reports for bugs, edge cases, and gaps.".into(),
            prompt_template: "Perform a thorough review of the relevant workspace files. Cite exact file paths and line numbers, highlight bugs or inconsistencies, and suggest minimal fixes via `propose_edit`.\n\nScope: {input}".into(),
            recommended_mode: AgentMode::Plan,
            is_builtin: true,
        },
        SkillDefinition {
            slug: "uni-solution".into(),
            name: "Uni Writeup".into(),
            description: "Format lab benchmarks, proofs, and plots into a clean academic Markdown/LaTeX/HTML report.".into(),
            prompt_template: "Inspect existing benchmark outputs, scripts, and PDFs in the workspace and produce a clear, well-structured academic writeup with exact observed metrics.\n\nAssignment: {input}".into(),
            recommended_mode: AgentMode::Agent,
            is_builtin: true,
        },
        SkillDefinition {
            slug: "verify".into(),
            name: "Verify & Self-Fix".into(),
            description: "Run build, syntax, or test checks with `verify_workspace` and fix any failures.".into(),
            prompt_template: "Run appropriate verification checks (`verify_workspace`) for this project and fix any compile, syntax, or test errors you uncover.\n\nTarget: {input}".into(),
            recommended_mode: AgentMode::Agent,
            is_builtin: true,
        },
        SkillDefinition {
            slug: "compact".into(),
            name: "Compact Summary".into(),
            description: "Compact earlier conversation history into a dense summary to keep context fast.".into(),
            prompt_template: "Provide a compact structured summary of everything accomplished so far in this session: key decisions, modified files, verified outputs, and remaining open items. {input}".into(),
            recommended_mode: AgentMode::Chat,
            is_builtin: true,
        },
        SkillDefinition {
            slug: "goal".into(),
            name: "Autonomous Goal".into(),
            description: "Run an extended multi-step execution loop (up to 12 autonomous steps) until verified.".into(),
            prompt_template: "Stay persistent and execute this goal end-to-end inside the workspace. Inspect files, plan steps via `write_artifact` (task_list), implement changes, run `verify_workspace`, and fix any errors until the goal is completely verified.\n\nGoal: {input}".into(),
            recommended_mode: AgentMode::Agent,
            is_builtin: true,
        },
        SkillDefinition {
            slug: "boost".into(),
            name: "Deep Boost".into(),
            description: "Activate deep reasoning effort with thorough dependency analysis and verification.".into(),
            prompt_template: "Use Deep reasoning depth. Investigate dependencies from multiple angles, track a checklist via `write_artifact`, verify all edits cleanly, and double-check edge cases.\n\nTask: {input}".into(),
            recommended_mode: AgentMode::Agent,
            is_builtin: true,
        },
        SkillDefinition {
            slug: "learn".into(),
            name: "Persist Learned Rule".into(),
            description: "Save a workspace preference or project convention persistently to .cinder/rules.md.".into(),
            prompt_template: "Record the following rule/convention into project memory and acknowledge what was persisted to `.cinder/rules.md`.\n\nRule: {input}".into(),
            recommended_mode: AgentMode::Chat,
            is_builtin: true,
        },
    ]
}

fn load_skills_from_dir(dir: &Path, skills: &mut Vec<SkillDefinition>) {
    if !dir.exists() {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let p = entry.path();
        // Flat `<slug>.md` format
        if p.is_file()
            && p.extension()
                .and_then(|e| e.to_str())
                .map(|e| e.eq_ignore_ascii_case("md"))
                .unwrap_or(false)
        {
            if let Ok(raw) = std::fs::read_to_string(&p) {
                let slug = p
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or("custom")
                    .to_string();
                let parsed = parse_skill_markdown(&slug, &raw);
                skills.retain(|s| s.slug != parsed.slug);
                skills.push(parsed);
            }
            continue;
        }
        // Antigravity standard `<slug>/SKILL.md` folder format
        if p.is_dir() {
            let skill_md = p.join("SKILL.md");
            if skill_md.is_file() {
                if let Ok(raw) = std::fs::read_to_string(&skill_md) {
                    let slug = p
                        .file_name()
                        .and_then(|s| s.to_str())
                        .unwrap_or("custom")
                        .to_string();
                    let parsed = parse_skill_markdown(&slug, &raw);
                    skills.retain(|s| s.slug != parsed.slug);
                    skills.push(parsed);
                }
            }
        }
    }
}

pub fn list_skills(workspace_root: &Path) -> Vec<SkillDefinition> {
    list_skills_with_global(workspace_root, None)
}

pub fn list_skills_with_global(
    workspace_root: &Path,
    global_skills_dir: Option<&Path>,
) -> Vec<SkillDefinition> {
    let mut skills = builtin_skills();
    if let Some(global_dir) = global_skills_dir {
        load_skills_from_dir(global_dir, &mut skills);
    }
    for dir_name in [".bumbo", ".cinder", ".agents"] {
        let custom_dir = workspace_root.join(dir_name).join("skills");
        load_skills_from_dir(&custom_dir, &mut skills);
    }
    skills
}

pub fn save_custom_skill(workspace_root: &Path, skill: &SkillDefinition) -> anyhow::Result<()> {
    let clean_slug: String = skill
        .slug
        .trim()
        .trim_start_matches('/')
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect();
    if clean_slug.is_empty() {
        anyhow::bail!("Skill slug cannot be empty.");
    }

    let dir = workspace_root.join(".cinder").join("skills");
    std::fs::create_dir_all(&dir)?;

    let mode_str = match skill.recommended_mode {
        AgentMode::Auto => "auto",
        AgentMode::Chat => "chat",
        AgentMode::Plan => "plan",
        AgentMode::Agent => "agent",
        AgentMode::Orchestrator => "orchestrator",
        AgentMode::Compare => "compare",
    };

    let content = format!(
        "---\nname: {}\ndescription: {}\nmode: {}\n---\n{}\n",
        skill.name.trim(),
        skill.description.trim(),
        mode_str,
        skill.prompt_template.trim()
    );
    std::fs::write(dir.join(format!("{clean_slug}.md")), content)?;
    Ok(())
}

pub fn expand_slash_skill(workspace_root: &Path, raw_input: &str) -> (String, Option<AgentMode>) {
    expand_slash_skill_with_global(workspace_root, None, raw_input)
}

pub fn expand_slash_skill_with_global(
    workspace_root: &Path,
    global_skills_dir: Option<&Path>,
    raw_input: &str,
) -> (String, Option<AgentMode>) {
    let trimmed = raw_input.trim();
    if !trimmed.starts_with('/') {
        return (raw_input.to_string(), None);
    }

    let mut parts = trimmed[1..].splitn(2, char::is_whitespace);
    let slug = parts.next().unwrap_or("").trim().to_ascii_lowercase();
    let rest = parts.next().unwrap_or("").trim();

    let skills = list_skills_with_global(workspace_root, global_skills_dir);
    if let Some(skill) = skills.into_iter().find(|s| s.slug.eq_ignore_ascii_case(&slug)) {
        let expanded = if skill.prompt_template.contains("{input}") {
            skill
                .prompt_template
                .replace("{input}", if rest.is_empty() { "current workspace" } else { rest })
        } else if rest.is_empty() {
            skill.prompt_template
        } else {
            format!("{}\n\n{rest}", skill.prompt_template)
        };
        return (expanded, Some(skill.recommended_mode));
    }

    (raw_input.to_string(), None)
}

fn parse_skill_markdown(slug: &str, raw: &str) -> SkillDefinition {
    let mut name = slug.to_string();
    let mut description = "Custom workspace skill".to_string();
    let mut mode = AgentMode::Plan;
    let mut body = raw.trim().to_string();

    if let Some(after_open) = raw.strip_prefix("---\n").or_else(|| raw.strip_prefix("---\r\n")) {
        if let Some(end_idx) = after_open.find("\n---") {
            let fm = &after_open[..end_idx];
            body = after_open[end_idx + 4..].trim().to_string();
            for line in fm.lines() {
                if let Some((k, v)) = line.split_once(':') {
                    let key = k.trim().to_ascii_lowercase();
                    let val = v.trim();
                    match key.as_str() {
                        "name" if !val.is_empty() => name = val.to_string(),
                        "description" if !val.is_empty() => description = val.to_string(),
                        "mode" => {
                            mode = match val.to_ascii_lowercase().as_str() {
                                "auto" => AgentMode::Auto,
                                "chat" => AgentMode::Chat,
                                "agent" => AgentMode::Agent,
                                "orchestrator" => AgentMode::Orchestrator,
                                "compare" => AgentMode::Compare,
                                _ => AgentMode::Plan,
                            };
                        }
                        _ => {}
                    }
                }
            }
        }
    }

    SkillDefinition {
        slug: slug.to_string(),
        name,
        description,
        prompt_template: body,
        recommended_mode: mode,
        is_builtin: false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expands_builtin_slash_skills() {
        let dir = std::env::temp_dir();
        let (exp1, mode1) = expand_slash_skill(&dir, "/plan refactor the parser");
        assert_eq!(mode1, Some(AgentMode::Plan));
        assert!(exp1.contains("refactor the parser"));
        assert!(exp1.contains("submit_plan"));

        let (exp2, mode2) = expand_slash_skill(&dir, "/grill-me choose database engine");
        assert_eq!(mode2, Some(AgentMode::Plan));
        assert!(exp2.contains("choose database engine"));
        assert!(exp2.contains("ask_user"));
    }
}
