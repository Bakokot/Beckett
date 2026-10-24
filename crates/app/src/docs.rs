//! Godot's documentation Markdown, simplified for popups.

const MAX_BODY_CHARS: usize = 1200;

/// A signature line (Godot puts it first, tab-indented) plus prose.
#[derive(Debug, Clone, PartialEq)]
pub struct Doc {
    pub signature: Option<String>,
    pub body: String,
}

impl Doc {
    pub fn from_markdown(markdown: &str) -> Self {
        let mut lines = markdown.lines().skip_while(|l| l.trim().is_empty()).peekable();
        let signature = lines
            .next_if(|l| l.starts_with('\t') || looks_like_code(l))
            .map(|l| clean_inline(l.trim()));

        let mut body = String::new();
        let mut blank = false;
        for line in lines {
            if line.trim_start().starts_with("```") {
                continue;
            }
            if line.trim().is_empty() {
                blank = !body.is_empty();
                continue;
            }
            if blank {
                body.push_str("\n\n");
            } else if !body.is_empty() {
                body.push('\n');
            }
            blank = false;
            body.push_str(&clean_inline(line.trim_end()));
        }
        if body.chars().count() > MAX_BODY_CHARS {
            body = body.chars().take(MAX_BODY_CHARS).collect::<String>() + "…";
        }
        Self { signature, body }
    }

    pub fn is_empty(&self) -> bool {
        self.signature.is_none() && self.body.is_empty()
    }
}

fn looks_like_code(line: &str) -> bool {
    ["func ", "var ", "const ", "signal ", "class ", "enum "]
        .iter()
        .any(|prefix| line.trim_start().starts_with(prefix))
}

fn clean_inline(line: &str) -> String {
    line.replace("**", "").replace('`', "").replace('\t', "    ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_signature_and_cleans_markdown() {
        let doc = Doc::from_markdown(
            "\tfunc @GlobalScope.print(...) -> void\n\nConverts **one** or more `args`.\n```gdscript\nprint(\"a\")\n```\n\n\nNext paragraph.\n",
        );
        assert_eq!(doc.signature.as_deref(), Some("func @GlobalScope.print(...) -> void"));
        assert_eq!(doc.body, "Converts one or more args.\nprint(\"a\")\n\nNext paragraph.");
    }

    #[test]
    fn prose_only_and_empty() {
        let doc = Doc::from_markdown("Just text.");
        assert_eq!(doc.signature, None);
        assert_eq!(doc.body, "Just text.");
        assert!(Doc::from_markdown("\n\n").is_empty());
    }
}
