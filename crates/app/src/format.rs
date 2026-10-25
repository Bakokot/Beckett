//! Formatting a document: `gdformat` (from gdtoolkit) for GDScript when it
//! is installed (Godot's language server has no formatter), else a light
//! built-in tidy for every file type.

use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};

/// Trailing spaces and tabs removed, exactly one newline at the end.
pub fn tidy(text: &str) -> String {
    let mut out: String = text.lines().map(|line| format!("{}\n", line.trim_end_matches([' ', '\t']))).collect();
    while out.ends_with("\n\n") {
        out.pop();
    }
    if out == "\n" {
        out.clear();
    }
    out
}

/// `gdformat` on the PATH, if installed.
pub fn gdformat_path() -> Option<PathBuf> {
    let names: &[&str] = if cfg!(windows) { &["gdformat.exe", "gdformat.cmd", "gdformat"] } else { &["gdformat"] };
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path).flat_map(|dir| names.iter().map(move |n| dir.join(n))).find(|p| p.is_file())
}

/// Formats GDScript with `gdformat` (reading stdin, writing stdout).
pub fn gdformat(exe: &std::path::Path, text: &str) -> Result<String, String> {
    let mut command = Command::new(exe);
    command.arg("-").stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    let mut child = command.spawn().map_err(|e| format!("Cannot run gdformat: {e}"))?;
    if let Some(mut stdin) = child.stdin.take() {
        stdin.write_all(text.as_bytes()).map_err(|e| e.to_string())?;
    }
    let output = child.wait_with_output().map_err(|e| e.to_string())?;
    if output.status.success() {
        String::from_utf8(output.stdout).map_err(|e| e.to_string())
    } else {
        let error = String::from_utf8_lossy(&output.stderr);
        Err(format!("gdformat: {}", error.lines().find(|l| !l.trim().is_empty()).unwrap_or("failed")))
    }
}

/// The smallest single edit turning `old` into `new`: char range of `old`
/// to replace, and the replacement. `None` when they are equal.
pub fn minimal_edit(old: &str, new: &str) -> Option<(usize, usize, String)> {
    if old == new {
        return None;
    }
    let (a, b): (Vec<char>, Vec<char>) = (old.chars().collect(), new.chars().collect());
    let prefix = a.iter().zip(&b).take_while(|(x, y)| x == y).count();
    let max_suffix = a.len().min(b.len()) - prefix;
    let suffix = a.iter().rev().zip(b.iter().rev()).take(max_suffix).take_while(|(x, y)| x == y).count();
    let replacement: String = b[prefix..b.len() - suffix].iter().collect();
    Some((prefix, a.len() - suffix, replacement))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tidy_trims_lines_and_ends_with_one_newline() {
        assert_eq!(tidy("func f():  \n\tpass\t\n\n\n"), "func f():\n\tpass\n");
        assert_eq!(tidy("a"), "a\n");
        assert_eq!(tidy("\n\n"), "");
        assert_eq!(tidy("x\n\ny\n"), "x\n\ny\n");
    }

    #[test]
    fn minimal_edit_replaces_only_the_changed_middle() {
        assert_eq!(minimal_edit("same", "same"), None);
        assert_eq!(minimal_edit("var x=1\n", "var x = 1\n"), Some((5, 6, " = ".into())));
        assert_eq!(minimal_edit("ab  \n", "ab\n"), Some((2, 4, String::new())));
        assert_eq!(minimal_edit("aaa", "aaaa"), Some((3, 3, "a".into())));
    }
}
