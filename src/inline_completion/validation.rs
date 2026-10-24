//! Validate the hypothetical insertion off-thread; never modify the document.
use crate::syntax::{self, Lang};

fn errors(root: tree_sitter::Node<'_>) -> (usize, usize) {
    let mut count = 0;
    let mut bytes = 0;
    let mut stack = vec![root];
    while let Some(node) = stack.pop() {
        if node.is_error() || node.is_missing() {
            count += 1;
            bytes += node.end_byte().saturating_sub(node.start_byte()) + 1;
            continue;
        }
        if node.has_error() {
            let mut cursor = node.walk();
            stack.extend(node.children(&mut cursor));
        }
    }
    (count, bytes)
}
/// A match-arm expression needs its separator before an existing next arm.
/// Models often omit it; add it only after a real Rust fat-arrow token in a
/// match block, never inside string/macro token trees or other languages.
pub fn normalize(lang: Lang, source: &str, at: usize, insertion: &str) -> String {
    if lang != Lang::Rust
        || insertion.is_empty()
        || at > source.len()
        || !source.is_char_boundary(at)
        || insertion.trim_end().ends_with(',')
        || source[at..].trim_start().starts_with(',')
    {
        return insertion.into();
    }
    let before = source[..at].trim_end();
    if !before.ends_with("=>") {
        return insertion.into();
    }
    let Some(grammar) = syntax::grammar(lang) else {
        return insertion.into();
    };
    let mut parser = tree_sitter::Parser::new();
    if parser.set_language(&grammar).is_err() {
        return insertion.into();
    }
    let Some(tree) = parser.parse(source, None) else {
        return insertion.into();
    };
    let Some(arrow) = tree.root_node().descendant_for_byte_range(before.len() - 1, before.len()) else {
        return insertion.into();
    };
    if arrow.kind() != "=>" {
        return insertion.into();
    }
    let mut node = arrow.parent();
    while let Some(parent) = node {
        if parent.kind() == "token_tree" {
            return insertion.into();
        }
        if parent.kind() == "match_block" {
            return format!("{},", insertion.trim_end());
        }
        node = parent.parent();
    }
    insertion.into()
}

pub fn acceptable(lang: Lang, source: &str, at: usize, insertion: &str) -> bool {
    if insertion.is_empty() {
        return true;
    }
    if at > source.len() || !source.is_char_boundary(at) {
        return false;
    }
    let Some(grammar) = syntax::grammar(lang) else {
        return true;
    };
    let mut parser = tree_sitter::Parser::new();
    if parser.set_language(&grammar).is_err() {
        return false;
    }
    let Some(before) = parser.parse(source, None) else {
        return false;
    };
    // Rust token trees accept arbitrary tokens, including nonsensical match
    // arms in string macros. A successful parse alone cannot reject these.
    if lang == Lang::Rust && insertion.contains("=>") && !insertion.contains("match ") {
        static ARM: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
        let arm = ARM.get_or_init(|| {
            // The pattern is a literal in this source; a failure is a build-time bug.
            regex::Regex::new(r#"(?m)^\s*(?:"[^"\n]*"|[A-Za-z_][A-Za-z_0-9:]*(?:\([^)]*\))?)\s*=>"#)
                .unwrap_or_else(|e| panic!("invalid match-arm regex: {e}"))
        });
        if arm.is_match(insertion) {
            let mut node = before.root_node().descendant_for_byte_range(at, at);
            let mut in_match = false;
            while let Some(current) = node {
                if matches!(current.kind(), "match_block" | "match_expression" | "macro_definition") {
                    in_match = true;
                    break;
                }
                node = current.parent();
            }
            if !in_match {
                return false;
            }
        }
    }
    let mut proposed = String::with_capacity(source.len() + insertion.len());
    proposed.push_str(&source[..at]);
    proposed.push_str(insertion);
    proposed.push_str(&source[at..]);
    let Some(after) = parser.parse(&proposed, None) else {
        return false;
    };
    let old = errors(before.root_node());
    let new = errors(after.root_node());
    new.0 <= old.0 && new.1 <= old.1
}
/// Checks a Next Edit replacement of `range` (bytes) against the whole
/// document. `acceptable` assumes a pure insertion, so it is not reused here.
/// Rejects replacements that add syntax errors; a warning means the result
/// parses no worse than before but cannot be fully vouched for.
pub fn check_replacement(
    lang: Lang,
    source: &str,
    range: std::ops::Range<usize>,
    replacement: &str,
) -> Result<Option<&'static str>, &'static str> {
    if range.start > range.end || range.end > source.len() || !source.is_char_boundary(range.start) || !source.is_char_boundary(range.end) {
        return Err("Next Edit region no longer matches the document");
    }
    let Some(grammar) = syntax::grammar(lang) else {
        return Ok(None);
    };
    let mut parser = tree_sitter::Parser::new();
    if parser.set_language(&grammar).is_err() {
        return Ok(Some("Syntax was not checked for this file"));
    }
    let proposed = format!("{}{}{}", &source[..range.start], replacement, &source[range.end..]);
    let (Some(before), Some(after)) = (parser.parse(source, None), parser.parse(&proposed, None)) else {
        return Ok(Some("Syntax was not checked for this file"));
    };
    let old = errors(before.root_node());
    let new = errors(after.root_node());
    if new.0 > old.0 {
        return Err("Next Edit rejected: the change would introduce syntax errors");
    }
    if new.1 > old.1 {
        return Ok(Some("The change moves or widens existing syntax errors; review it carefully"));
    }
    if new.0 > 0 {
        return Ok(Some("The file still has syntax errors, so the check is incomplete"));
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn replacement_check_rejects_new_errors_and_warns_when_ambiguous() {
        let source = "fn a() {\n    old();\n}\n";
        let at = source.find("old").unwrap_or_else(|| panic!("expected \"old\""));
        let range = at..at + "old();".len();
        assert_eq!(check_replacement(Lang::Rust, source, range.clone(), "new();"), Ok(None));
        // Deletion is a replacement too.
        assert_eq!(check_replacement(Lang::Rust, source, range.clone(), ""), Ok(None));
        assert!(check_replacement(Lang::Rust, source, range.clone(), "new(;\n}\nfn b( {").is_err());
        let broken = "fn a() {\n    old(;\n}\n";
        let at = broken.find("old").unwrap_or_else(|| panic!("expected \"old\""));
        assert!(matches!(check_replacement(Lang::Rust, broken, at..at + 3, "new"), Ok(Some(_))));
        assert_eq!(check_replacement(Lang::Plain, "x", 0..1, "(("), Ok(None));
        assert!(check_replacement(Lang::Rust, "😀", 1..2, "x").is_err());
    }
    #[test]
    fn rejects_match_arms_inside_help_expression() {
        let source = "fn help() -> &'static str {\n    t_file!(\"cli-help.txt\")\n}\nfn run() { match command { _ => () } }";
        let at = source.find("\n}").unwrap_or_else(|| panic!("expected newline-brace"));
        assert!(!acceptable(Lang::Rust, source, at, "\"command\" => Some(call_and_print()),\n\"open\" => open(),"));
        assert!(acceptable(Lang::Rust, source, at, ""));
    }
    #[test]
    fn rejects_match_arms_inside_macro_arguments() {
        let source = "fn help() -> &'static str { t_file!(\"cli-help.txt\") }";
        let at = source.find("txt\"").unwrap_or_else(|| panic!("expected text marker")) + 4;
        assert!(!acceptable(Lang::Rust, source, at, "\"command\" => command(),\n\"open\" => open(),"));
        let source = "fn run() { match command {  } }";
        let at = source.find("{  }").unwrap_or_else(|| panic!("expected empty block")) + 2;
        assert!(acceptable(Lang::Rust, source, at, "\"open\" => open(),"));
    }
    #[test]
    fn fixes_incomplete_expression_and_keeps_valid_blocks() {
        let source = "fn main() { client. }";
        let at = source.find("client.").unwrap_or_else(|| panic!("expected client.")) + 7;
        assert!(acceptable(Lang::Rust, source, at, "send()"));
        assert!(!acceptable(Lang::Rust, source, at, "send(\nfn unrelated() {}"));
        let source = "fn square(x:i32)->i32 {\n    \n}";
        let at = source.find("    ").unwrap_or_else(|| panic!("expected indentation")) + 4;
        assert!(acceptable(Lang::Rust, source, at, "let result=x*x;\n    result"));
        let source = "def square(x):\n    \n";
        assert!(acceptable(Lang::Python, source, source.len() - 1, "return x*x"));
    }
    #[test]
    fn completes_missing_match_arm_body_before_existing_next_arm() {
        let source = "fn run(command:&str) { match command {\n    \"close\" => \n    \"help\" => help(),\n    _ => (),\n} }";
        let at = source.find("=> ").unwrap_or_else(|| panic!("expected \"=> \"")) + 3;
        assert!(acceptable(Lang::Rust, source, at, "close(),"));
        assert!(acceptable(Lang::Rust, source, at, "{ close(); },"));
    }
    #[test]
    fn inserts_missing_match_separator_without_touching_macro_tokens() {
        let source = "fn run(x:&str) { match x { \"close\" => \n\"help\" => help(), _ => () } }";
        let at = source.find("=> ").unwrap_or_else(|| panic!("expected \"=> \"")) + 3;
        let text = normalize(Lang::Rust, source, at, "close()");
        assert_eq!(text, "close(),");
        let proposed = format!("{}{}{}", &source[..at], text, &source[at..]);
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&syntax::grammar(Lang::Rust).unwrap_or_else(|| panic!("rust grammar missing")))
            .unwrap_or_else(|e| panic!("set_language failed: {e}"));
        assert!(!parser.parse(&proposed, None).unwrap_or_else(|| panic!("parse produced no tree")).root_node().has_error());
        assert_eq!(normalize(Lang::Rust, source, at, "close(),"), "close(),");
        let source = "fn f() { macro_call!(a => ); }";
        let at = source.find("=> ").unwrap_or_else(|| panic!("expected \"=> \"")) + 3;
        assert_eq!(normalize(Lang::Rust, source, at, "something()"), "something()");
    }
    #[test]
    fn utf8_and_unknown_languages() {
        assert!(!acceptable(Lang::Rust, "😀", 1, "x"));
        assert!(acceptable(Lang::Plain, "", 0, "hello"));
    }
}
