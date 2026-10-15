//! Problems in a `deploy.yaml`, and how they are printed when shipdeck
//! refuses to start: one block per problem with the line it is on, a caret
//! under the spot and a hint, in colour on a terminal.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

/// Where something was written: 1-based lines, 0-based columns, in
/// characters. `end` is exclusive.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Span {
    pub start: (usize, usize),
    pub end: (usize, usize),
}

impl Span {
    pub fn point(line: usize, col: usize) -> Self {
        Self {
            start: (line, col),
            end: (line, col),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Problem {
    /// What is wrong. Key paths go in backticks; they print in colour.
    pub message: String,
    pub at: Option<Span>,
    /// Printed after the carets.
    pub label: String,
    /// How to put it right.
    pub help: Option<String>,
}

impl Problem {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            at: None,
            label: String::new(),
            help: None,
        }
    }

    pub fn at(mut self, span: Option<Span>) -> Self {
        self.at = span;
        self
    }

    pub fn label(mut self, label: impl Into<String>) -> Self {
        self.label = label.into();
        self
    }

    pub fn help(mut self, help: impl Into<String>) -> Self {
        self.help = Some(help.into());
        self
    }
}

impl std::fmt::Display for Problem {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.at {
            Some(at) => write!(f, "line {}: {}", at.start.0, self.message)?,
            None => write!(f, "{}", self.message)?,
        }
        if let Some(help) = &self.help {
            write!(f, " ({help})")?;
        }
        Ok(())
    }
}

/// Every problem in one file.
#[derive(Debug, Clone)]
pub struct FileProblems {
    pub path: PathBuf,
    /// The file's text, for the snippets; `None` when it could not be read.
    pub source: Option<String>,
    pub problems: Vec<Problem>,
}

impl FileProblems {
    pub fn without_source(path: PathBuf, problem: Problem) -> Self {
        Self {
            path,
            source: None,
            problems: vec![problem],
        }
    }
}

struct Paint {
    on: bool,
}

impl Paint {
    fn wrap(&self, code: &str, text: &str) -> String {
        if self.on {
            format!("\x1b[{code}m{text}\x1b[0m")
        } else {
            text.to_string()
        }
    }
    fn red(&self, t: &str) -> String {
        self.wrap("1;31", t)
    }
    fn bold(&self, t: &str) -> String {
        self.wrap("1", t)
    }
    fn blue(&self, t: &str) -> String {
        self.wrap("34", t)
    }
    fn yellow(&self, t: &str) -> String {
        self.wrap("1;33", t)
    }
    fn green(&self, t: &str) -> String {
        self.wrap("1;32", t)
    }
    fn purple(&self, t: &str) -> String {
        self.wrap("35", t)
    }

    /// Text with its backticked key paths in purple. Without colour the
    /// backticks stay, so the paths still stand out.
    fn keys(&self, text: &str, base: &str) -> String {
        if !self.on {
            return text.to_string();
        }
        let mut out = String::new();
        for (i, part) in text.split('`').enumerate() {
            if i % 2 == 1 {
                out.push_str(&self.purple(part));
                if !base.is_empty() {
                    out.push_str(&format!("\x1b[{base}m"));
                }
            } else {
                out.push_str(part);
            }
        }
        out
    }
}

/// The whole report shipdeck prints before refusing to go on. `base` is
/// the folder paths are shown relative to; `color` adds terminal colours.
/// `outcome` heads the report: what did not happen because of them, such
/// as `shipdeck did not start`.
pub fn render(files: &[FileProblems], base: &Path, color: bool, outcome: &str) -> String {
    let paint = Paint { on: color };
    let count: usize = files.iter().map(|f| f.problems.len()).sum();
    let mut out = String::new();
    let files_text = if files.len() == 1 {
        "1 deploy.yaml file has".to_string()
    } else {
        format!("{} deploy.yaml files have", files.len())
    };
    let problems_text = if count == 1 {
        "1 problem".to_string()
    } else {
        format!("{count} problems")
    };
    let _ = writeln!(
        out,
        "{} {}",
        paint.red("✘"),
        paint.bold(&format!("{outcome}: {files_text} {problems_text}"))
    );
    for file in files {
        let shown = file.path.strip_prefix(base).unwrap_or(&file.path);
        let short = short_name(&file.path);
        let _ = writeln!(out, "\n{}", paint.bold(&short));
        let lines: Vec<&str> = file
            .source
            .as_deref()
            .map(|s| s.lines().collect())
            .unwrap_or_default();
        for (index, problem) in file.problems.iter().enumerate() {
            if index > 0 {
                out.push('\n');
            }
            snippet(&mut out, &paint, shown, &lines, problem);
        }
    }
    out
}

/// `web-prod/deploy.yaml`: the deployable's folder and the file.
fn short_name(path: &Path) -> String {
    let file = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    match path.parent().and_then(Path::file_name) {
        Some(folder) => format!("{}/{file}", folder.to_string_lossy()),
        None => file,
    }
}

fn snippet(out: &mut String, paint: &Paint, path: &Path, lines: &[&str], problem: &Problem) {
    let _ = writeln!(
        out,
        "  {}{} {}",
        paint.red("error"),
        paint.bold(":"),
        paint.wrap("1", &paint.keys(&problem.message, "1"))
    );
    let at = problem
        .at
        .filter(|at| at.start.0 >= 1 && at.start.0 <= lines.len());
    let line_no = at.map(|at| at.start.0).unwrap_or(0);
    let width = line_no.to_string().len().max(2);
    let pad = " ".repeat(width + 2);
    let bar = paint.blue("│");
    match at {
        Some(at) => {
            let _ = writeln!(
                out,
                "{pad}{} {}:{}:{}",
                paint.blue("┌─"),
                path.display(),
                at.start.0,
                at.start.1 + 1
            );
            let _ = writeln!(out, "{pad}{bar}");
            let line = lines[at.start.0 - 1];
            // The line above, when it is the key this one sits under.
            if at.start.0 >= 2 {
                let above = lines[at.start.0 - 2];
                let indent = |s: &str| s.len() - s.trim_start().len();
                let trimmed = above.trim_start();
                if !trimmed.is_empty()
                    && !trimmed.starts_with('#')
                    && trimmed.trim_end().ends_with(':')
                    && indent(above) < indent(line)
                {
                    let _ = writeln!(
                        out,
                        "{} {bar} {above}",
                        paint.blue(&format!(" {:>w$}", at.start.0 - 1, w = width))
                    );
                }
            }
            let _ = writeln!(
                out,
                "{} {bar} {line}",
                paint.blue(&format!(" {:>w$}", at.start.0, w = width))
            );
            let col = at.start.1.min(line.chars().count());
            let len = if at.end.0 == at.start.0 && at.end.1 > at.start.1 {
                at.end.1 - at.start.1
            } else {
                line.chars().count().saturating_sub(col)
            }
            .max(1);
            let label = if problem.label.is_empty() {
                String::new()
            } else {
                format!(" {}", problem.label)
            };
            let _ = writeln!(
                out,
                "{pad}{bar} {}{}",
                " ".repeat(col),
                paint.yellow(&format!("{}{label}", "^".repeat(len)))
            );
            let _ = writeln!(out, "{pad}{bar}");
        }
        None => {
            let _ = writeln!(out, "{pad}{} {}", paint.blue("┌─"), path.display());
            let _ = writeln!(out, "{pad}{bar}");
        }
    }
    if let Some(help) = &problem.help {
        let _ = writeln!(
            out,
            "{pad}{} {}{} {}",
            paint.blue("="),
            paint.green("help"),
            paint.bold(":"),
            paint.keys(help, "")
        );
    }
}
