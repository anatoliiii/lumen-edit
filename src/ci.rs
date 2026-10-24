//! CI/CD and infrastructure files: platform detection by path, icons, and
//! an overlay that highlights pipeline keys, job names and expressions on
//! top of the YAML grammar.

use std::ops::Range;

use eframe::egui::Color32;

use crate::syntax::{HIGHLIGHT_NAMES, Span};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ci {
    GitLab,
    GitHub,
    Gitea,
    Forgejo,
    Woodpecker,
    Drone,
    Bitbucket,
    Azure,
    CircleCi,
    Travis,
    Jenkins,
    Dockerfile,
    Compose,
    Kubernetes,
    Ansible,
    Terraform,
    PreCommit,
    Dependabot,
}

impl Ci {
    pub fn detect(path: &str) -> Option<Ci> {
        let lower = path.to_ascii_lowercase();
        let name = lower.rsplit('/').next().unwrap_or(&lower);
        let yaml = name.ends_with(".yml") || name.ends_with(".yaml");
        let inside = |dir: &str| lower.contains(dir);
        Some(match name {
            ".gitlab-ci.yml" | ".gitlab-ci.yaml" => Ci::GitLab,
            ".drone.yml" | ".drone.yaml" => Ci::Drone,
            ".woodpecker.yml" | ".woodpecker.yaml" => Ci::Woodpecker,
            "bitbucket-pipelines.yml" => Ci::Bitbucket,
            "azure-pipelines.yml" | "azure-pipelines.yaml" => Ci::Azure,
            ".travis.yml" => Ci::Travis,
            "jenkinsfile" => Ci::Jenkins,
            ".pre-commit-config.yaml" | ".pre-commit-config.yml" => Ci::PreCommit,
            "dependabot.yml" | "dependabot.yaml" if inside("/.github/") => Ci::Dependabot,
            "dockerfile" | "containerfile" => Ci::Dockerfile,
            _ if name.starts_with("dockerfile.") || name.ends_with(".dockerfile") => Ci::Dockerfile,
            _ if name.starts_with("docker-compose") || name == "compose.yml" || name == "compose.yaml" => Ci::Compose,
            _ if name.ends_with(".tf") || name.ends_with(".tfvars") || name.ends_with(".hcl") => Ci::Terraform,
            _ if yaml && inside("/.github/workflows/") => Ci::GitHub,
            _ if yaml && inside("/.gitea/workflows/") => Ci::Gitea,
            _ if yaml && inside("/.forgejo/workflows/") => Ci::Forgejo,
            _ if yaml && inside("/.woodpecker/") => Ci::Woodpecker,
            _ if yaml && inside("/.circleci/") => Ci::CircleCi,
            _ if yaml && (inside("/.gitlab/ci/") || name.ends_with(".gitlab-ci.yml")) => Ci::GitLab,
            _ if yaml && (inside("/k8s/") || inside("/kubernetes/") || inside("/manifests/") || inside("/helm/")) => Ci::Kubernetes,
            _ if yaml && (inside("/playbooks/") || inside("/roles/") || name.starts_with("playbook")) => Ci::Ansible,
            _ => return None,
        })
    }

    pub fn label(self) -> &'static str {
        match self {
            Ci::GitLab => "GitLab CI",
            Ci::GitHub => "GitHub Actions",
            Ci::Gitea => "Gitea Actions",
            Ci::Forgejo => "Forgejo Actions",
            Ci::Woodpecker => "Woodpecker CI",
            Ci::Drone => "Drone CI",
            Ci::Bitbucket => "Bitbucket Pipelines",
            Ci::Azure => "Azure Pipelines",
            Ci::CircleCi => "CircleCI",
            Ci::Travis => "Travis CI",
            Ci::Jenkins => "Jenkinsfile",
            Ci::Dockerfile => "Dockerfile",
            Ci::Compose => "Docker Compose",
            Ci::Kubernetes => "Kubernetes",
            Ci::Ansible => "Ansible",
            Ci::Terraform => "Terraform",
            Ci::PreCommit => "pre-commit",
            Ci::Dependabot => "Dependabot",
        }
    }

    /// Short badge and brand-like color for trees and tabs.
    pub fn icon(self) -> (&'static str, Color32) {
        match self {
            Ci::GitLab => ("⚑", Color32::from_rgb(226, 103, 54)),
            Ci::GitHub => ("⏵", Color32::from_rgb(200, 205, 215)),
            Ci::Gitea => ("⏵", Color32::from_rgb(96, 153, 38)),
            Ci::Forgejo => ("⏵", Color32::from_rgb(251, 146, 60)),
            Ci::Woodpecker => ("⏵", Color32::from_rgb(76, 175, 80)),
            Ci::Drone => ("⏵", Color32::from_rgb(33, 150, 243)),
            Ci::Bitbucket => ("⏵", Color32::from_rgb(38, 132, 255)),
            Ci::Azure => ("⏵", Color32::from_rgb(0, 120, 212)),
            Ci::CircleCi => ("◉", Color32::from_rgb(200, 205, 215)),
            Ci::Travis => ("⏵", Color32::from_rgb(205, 50, 77)),
            Ci::Jenkins => ("J", Color32::from_rgb(213, 72, 72)),
            Ci::Dockerfile | Ci::Compose => ("◈", Color32::from_rgb(29, 99, 237)),
            Ci::Kubernetes => ("⎈", Color32::from_rgb(50, 108, 229)),
            Ci::Ansible => ("A", Color32::from_rgb(220, 220, 220)),
            Ci::Terraform => ("◆", Color32::from_rgb(132, 79, 186)),
            Ci::PreCommit => ("✓", Color32::from_rgb(250, 176, 5)),
            Ci::Dependabot => ("↻", Color32::from_rgb(3, 102, 214)),
        }
    }

    /// Keys of the pipeline schema, highlighted as keywords.
    fn keywords(self) -> &'static [&'static str] {
        match self {
            Ci::GitLab => &[
                "stages",
                "stage",
                "variables",
                "include",
                "default",
                "workflow",
                "image",
                "services",
                "before_script",
                "after_script",
                "script",
                "rules",
                "if",
                "when",
                "only",
                "except",
                "needs",
                "dependencies",
                "artifacts",
                "paths",
                "expire_in",
                "reports",
                "cache",
                "key",
                "policy",
                "tags",
                "allow_failure",
                "retry",
                "timeout",
                "parallel",
                "matrix",
                "trigger",
                "extends",
                "environment",
                "resource_group",
                "interruptible",
                "changes",
                "exists",
                "coverage",
                "release",
                "pages",
                "secrets",
                "id_tokens",
                "inherit",
                "local",
                "project",
                "ref",
                "file",
                "template",
                "remote",
                "component",
                "inputs",
                "spec",
            ],
            Ci::GitHub | Ci::Gitea | Ci::Forgejo => &[
                "name",
                "run-name",
                "on",
                "push",
                "pull_request",
                "pull_request_target",
                "workflow_dispatch",
                "schedule",
                "cron",
                "release",
                "branches",
                "tags",
                "paths",
                "types",
                "permissions",
                "env",
                "defaults",
                "concurrency",
                "group",
                "cancel-in-progress",
                "jobs",
                "runs-on",
                "needs",
                "if",
                "steps",
                "uses",
                "with",
                "run",
                "shell",
                "working-directory",
                "id",
                "strategy",
                "matrix",
                "include",
                "exclude",
                "fail-fast",
                "max-parallel",
                "outputs",
                "services",
                "container",
                "timeout-minutes",
                "continue-on-error",
                "environment",
                "secrets",
                "inputs",
                "workflow_call",
                "image",
                "ports",
                "volumes",
            ],
            Ci::Woodpecker | Ci::Drone => &[
                "kind",
                "type",
                "name",
                "steps",
                "pipeline",
                "image",
                "commands",
                "environment",
                "when",
                "event",
                "branch",
                "depends_on",
                "settings",
                "secrets",
                "volumes",
                "services",
                "platform",
                "clone",
                "trigger",
                "matrix",
                "labels",
                "workspace",
                "path",
            ],
            Ci::Bitbucket | Ci::Azure | Ci::CircleCi | Ci::Travis => &[
                "pipelines",
                "pipeline",
                "default",
                "branches",
                "step",
                "steps",
                "script",
                "image",
                "caches",
                "services",
                "trigger",
                "pool",
                "vmImage",
                "stages",
                "stage",
                "jobs",
                "job",
                "variables",
                "parameters",
                "version",
                "workflows",
                "executors",
                "orbs",
                "commands",
                "run",
                "checkout",
                "language",
                "install",
                "before_install",
                "deploy",
                "matrix",
                "env",
            ],
            Ci::Compose => &[
                "services",
                "version",
                "image",
                "build",
                "context",
                "dockerfile",
                "ports",
                "volumes",
                "environment",
                "env_file",
                "networks",
                "depends_on",
                "restart",
                "command",
                "entrypoint",
                "healthcheck",
                "labels",
                "deploy",
                "secrets",
                "configs",
                "profiles",
            ],
            Ci::Kubernetes => &[
                "apiVersion",
                "kind",
                "metadata",
                "name",
                "namespace",
                "labels",
                "annotations",
                "spec",
                "replicas",
                "selector",
                "template",
                "containers",
                "image",
                "ports",
                "containerPort",
                "env",
                "resources",
                "limits",
                "requests",
                "volumes",
                "volumeMounts",
                "data",
                "type",
                "rules",
                "host",
                "http",
                "backend",
            ],
            Ci::Ansible => &[
                "hosts",
                "tasks",
                "name",
                "become",
                "vars",
                "vars_files",
                "roles",
                "handlers",
                "notify",
                "when",
                "loop",
                "with_items",
                "register",
                "tags",
                "gather_facts",
                "block",
                "rescue",
                "always",
            ],
            _ => &[],
        }
    }
}

fn index(name: &str) -> u8 {
    HIGHLIGHT_NAMES.iter().position(|n| *n == name).unwrap_or(0) as u8
}

/// Extra spans for CI YAML: schema keys, GitLab job names, `${{ }}`
/// expressions and `$VARIABLES`.
pub fn overlay(ci: Ci, text: &str) -> Vec<Span> {
    let keyword = index("keyword");
    let job = index("function");
    let embedded = index("embedded");
    let variable = index("variable.builtin");
    let keywords = ci.keywords();
    let mut spans = Vec::new();
    let mut offset = 0;
    for line in text.split_inclusive('\n') {
        let content = line.trim_end_matches(['\n', '\r']);
        let trimmed = content.trim_start();
        let indent = content.len() - trimmed.len();
        if !trimmed.starts_with('#') {
            // `key:` at the start of a mapping line (also after "- ").
            let key_start = indent + if trimmed.starts_with("- ") { 2 } else { 0 };
            let rest = &content[key_start..];
            if let Some(colon) = rest.find(':') {
                let key = rest[..colon].trim_matches(['"', '\'']);
                let simple = !key.is_empty() && key.chars().all(|c| c.is_alphanumeric() || "_-.".contains(c));
                if simple && rest[colon + 1..].chars().next().is_none_or(|c| c == ' ' || c == '\t') {
                    let range = offset + key_start..offset + key_start + colon;
                    if keywords.contains(&key) {
                        spans.push(Span { start: range.start, end: range.end, highlight: keyword });
                    } else if ci == Ci::GitLab && indent == 0 && !key.starts_with('.') {
                        // Top-level keys that are not keywords are job names.
                        spans.push(Span { start: range.start, end: range.end, highlight: job });
                    } else if indent == 2 && matches!(ci, Ci::GitHub | Ci::Gitea | Ci::Forgejo) && in_jobs_block(text, offset) {
                        spans.push(Span { start: range.start, end: range.end, highlight: job });
                    }
                }
            }
            for range in expressions(content) {
                spans.push(Span { start: offset + range.start, end: offset + range.end, highlight: embedded });
            }
            for range in variables(content) {
                spans.push(Span { start: offset + range.start, end: offset + range.end, highlight: variable });
            }
        }
        offset += line.len();
    }
    spans.sort_by_key(|s| s.start);
    spans
}

/// Whether the line at `offset` is inside the top-level `jobs:` mapping.
fn in_jobs_block(text: &str, offset: usize) -> bool {
    text[..offset]
        .lines()
        .rev()
        .find(|l| !l.starts_with(' ') && !l.trim().is_empty() && !l.starts_with('#'))
        .is_some_and(|l| l.starts_with("jobs:"))
}

/// `${{ ... }}` expressions (GitHub, Gitea, Forgejo).
fn expressions(line: &str) -> Vec<Range<usize>> {
    let mut found = Vec::new();
    let mut from = 0;
    while let Some(start) = line[from..].find("${{").map(|i| i + from) {
        let end = line[start..].find("}}").map(|i| start + i + 2).unwrap_or(line.len());
        found.push(start..end);
        from = end;
    }
    found
}

/// `$VAR` and `${VAR}` references (shell, GitLab, Woodpecker, Drone).
fn variables(line: &str) -> Vec<Range<usize>> {
    let bytes = line.as_bytes();
    let mut found = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'$' && i + 1 < bytes.len() {
            if bytes[i + 1] == b'{' && !line[i..].starts_with("${{") {
                if let Some(end) = line[i..].find('}') {
                    found.push(i..i + end + 1);
                    i += end + 1;
                    continue;
                }
            } else if bytes[i + 1].is_ascii_alphabetic() || bytes[i + 1] == b'_' {
                let mut end = i + 1;
                while end < bytes.len() && (bytes[end].is_ascii_alphanumeric() || bytes[end] == b'_') {
                    end += 1;
                }
                found.push(i..end);
                i = end;
                continue;
            }
        }
        i += 1;
    }
    found
}

/// Dockerfile highlighting without a grammar: instructions, comments,
/// strings, variables and `--flags`.
pub fn dockerfile(text: &str) -> Vec<Span> {
    const INSTRUCTIONS: &[&str] = &[
        "FROM",
        "RUN",
        "CMD",
        "LABEL",
        "MAINTAINER",
        "EXPOSE",
        "ENV",
        "ADD",
        "COPY",
        "ENTRYPOINT",
        "VOLUME",
        "USER",
        "WORKDIR",
        "ARG",
        "ONBUILD",
        "STOPSIGNAL",
        "HEALTHCHECK",
        "SHELL",
        "AS",
    ];
    let mut spans = Vec::new();
    let mut offset = 0;
    for line in text.split_inclusive('\n') {
        let content = line.trim_end_matches(['\n', '\r']);
        let trimmed = content.trim_start();
        let indent = content.len() - trimmed.len();
        if trimmed.starts_with('#') {
            spans.push(Span { start: offset + indent, end: offset + content.len(), highlight: index("comment") });
        } else {
            let mut position = indent;
            for word in trimmed.split_inclusive(char::is_whitespace) {
                let token = word.trim_end();
                let upper = token.to_ascii_uppercase();
                if INSTRUCTIONS.contains(&upper.as_str()) && (position == indent || upper == "AS") {
                    spans.push(Span { start: offset + position, end: offset + position + token.len(), highlight: index("keyword") });
                } else if token.starts_with("--") {
                    spans.push(Span { start: offset + position, end: offset + position + token.len(), highlight: index("attribute") });
                }
                position += word.len();
            }
            let mut quote: Option<(usize, u8)> = None;
            for (i, b) in content.bytes().enumerate() {
                match quote {
                    None if b == b'"' || b == b'\'' => quote = Some((i, b)),
                    Some((start, q)) if b == q => {
                        spans.push(Span { start: offset + start, end: offset + i + 1, highlight: index("string") });
                        quote = None;
                    }
                    _ => {}
                }
            }
            for range in variables(content) {
                spans.push(Span { start: offset + range.start, end: offset + range.end, highlight: index("variable.builtin") });
            }
        }
        offset += line.len();
    }
    merge(Vec::new(), spans)
}

/// Overlays `top` on `base`: overlapping parts of base spans are cut out.
pub fn merge(base: Vec<Span>, mut top: Vec<Span>) -> Vec<Span> {
    top.sort_by_key(|s| s.start);
    // Drop overlaps inside `top` itself, keeping the earlier span.
    let mut cleaned: Vec<Span> = Vec::with_capacity(top.len());
    for span in top {
        if cleaned.last().is_none_or(|last| span.start >= last.end) {
            cleaned.push(span);
        }
    }
    let mut out = Vec::with_capacity(base.len() + cleaned.len());
    let mut j = 0;
    for span in base {
        let mut start = span.start;
        while j < cleaned.len() && cleaned[j].end <= start {
            j += 1;
        }
        let mut k = j;
        while start < span.end {
            match cleaned.get(k) {
                Some(cut) if cut.start < span.end => {
                    if cut.start > start {
                        out.push(Span { start, end: cut.start, highlight: span.highlight });
                    }
                    start = start.max(cut.end);
                    k += 1;
                }
                _ => {
                    out.push(Span { start, end: span.end, highlight: span.highlight });
                    break;
                }
            }
        }
    }
    out.extend(cleaned);
    out.sort_by_key(|s| s.start);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_platforms() {
        assert_eq!(Ci::detect("/p/.gitlab-ci.yml"), Some(Ci::GitLab));
        assert_eq!(Ci::detect("/p/.github/workflows/ci.yml"), Some(Ci::GitHub));
        assert_eq!(Ci::detect("/p/.gitea/workflows/build.yaml"), Some(Ci::Gitea));
        assert_eq!(Ci::detect("/p/.forgejo/workflows/a.yml"), Some(Ci::Forgejo));
        assert_eq!(Ci::detect("/p/.woodpecker/test.yml"), Some(Ci::Woodpecker));
        assert_eq!(Ci::detect("/p/Jenkinsfile"), Some(Ci::Jenkins));
        assert_eq!(Ci::detect("/p/Dockerfile.dev"), Some(Ci::Dockerfile));
        assert_eq!(Ci::detect("/p/docker-compose.prod.yml"), Some(Ci::Compose));
        assert_eq!(Ci::detect("/p/main.tf"), Some(Ci::Terraform));
        assert_eq!(Ci::detect("/p/config.yml"), None);
    }

    #[test]
    fn gitlab_overlay() {
        let text = "stages:\n  - build\nbuild-job:\n  stage: build\n  script:\n    - echo $CI_COMMIT_SHA ${HOME}\n";
        let spans = overlay(Ci::GitLab, text);
        let at = |s: &str| text.find(s).unwrap();
        assert!(spans.iter().any(|s| s.start == at("stages") && s.highlight == index("keyword")));
        assert!(spans.iter().any(|s| s.start == at("build-job") && s.highlight == index("function")));
        assert!(spans.iter().any(|s| s.start == at("$CI_COMMIT_SHA") && s.end == at(" ${HOME}")));
        assert!(spans.iter().any(|s| s.start == at("${HOME}") && s.highlight == index("variable.builtin")));
    }

    #[test]
    fn github_expressions_and_jobs() {
        let text = "on: push\njobs:\n  test:\n    runs-on: ubuntu-latest\n    steps:\n      - run: echo ${{ github.sha }}\n";
        let spans = overlay(Ci::GitHub, text);
        let at = |s: &str| text.find(s).unwrap();
        assert!(spans.iter().any(|s| s.start == at("test:") && s.highlight == index("function")));
        assert!(spans.iter().any(|s| s.start == at("${{") && s.end == text.find("}}").unwrap() + 2));
    }

    #[test]
    fn merge_cuts_base() {
        let base = vec![Span { start: 0, end: 10, highlight: 1 }];
        let top = vec![Span { start: 3, end: 5, highlight: 2 }];
        let merged = merge(base, top);
        let ranges: Vec<_> = merged.iter().map(|s| (s.start, s.end, s.highlight)).collect();
        assert_eq!(ranges, vec![(0, 3, 1), (3, 5, 2), (5, 10, 1)]);
    }

    #[test]
    fn dockerfile_instructions() {
        let text = "# base\nFROM rust:1.88 AS build\nRUN cargo build --release\n";
        let spans = dockerfile(text);
        assert!(spans.iter().any(|s| s.start == text.find("FROM").unwrap() && s.highlight == index("keyword")));
        assert!(spans.iter().any(|s| s.start == text.find("AS").unwrap()));
        assert!(spans.iter().any(|s| s.start == 0 && s.highlight == index("comment")));
    }
}
