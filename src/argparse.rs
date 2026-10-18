//! The slice of Python 3.15's argparse that `lamp` uses: long options with
//! abbreviations, store and store_true actions, typed values, required
//! positionals and one level of subcommands. Errors, usage and help are
//! formatted as argparse formats them (never coloured).

use std::collections::HashMap;
use std::sync::LazyLock;

use fancy_regex::Regex;

use crate::pystr;

/// A type function: Ok(value) or Err(None) for "invalid NAME value", or
/// Err(Some(message)) for an ArgumentTypeError.
pub type Convert = fn(&str) -> Result<Value, Option<String>>;

#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Str(String),
    Int(i128),
    Float(f64),
    Bool(bool),
}

#[derive(Clone)]
pub enum Kind {
    Help,
    StoreTrue,
    Store {
        convert: Convert,
        type_name: &'static str,
    },
    Positional {
        convert: Convert,
        type_name: &'static str,
    },
    Subparsers {
        choices: Vec<(String, Option<String>, Parser)>,
    },
}

#[derive(Clone)]
pub struct Action {
    pub option_strings: Vec<String>,
    pub dest: String,
    pub help: Option<String>,
    pub metavar: Option<String>,
    pub default: Option<Value>,
    pub kind: Kind,
}

#[derive(Clone)]
pub struct Parser {
    pub prog: String,
    pub description: Option<String>,
    pub actions: Vec<Action>,
}

/// How a parse ended without a namespace: text for stdout or stderr, and
/// the exit status.
#[derive(Debug, PartialEq)]
pub struct Exit {
    pub code: i32,
    pub stdout: String,
    pub stderr: String,
}

pub type Namespace = HashMap<String, Value>;

/// A candidate reading of an option string: (action index, option string,
/// explicit argument, whether the argument came after `=`).
type OptionTuple = (Option<usize>, String, Option<String>, bool);

impl Parser {
    pub fn new(prog: &str, description: Option<&str>) -> Parser {
        Parser {
            prog: prog.into(),
            description: description.map(str::to_string),
            actions: vec![Action {
                option_strings: vec!["-h".into(), "--help".into()],
                dest: "help".into(),
                help: Some("show this help message and exit".into()),
                metavar: None,
                default: None,
                kind: Kind::Help,
            }],
        }
    }

    pub fn store(
        &mut self,
        option: &str,
        convert: Convert,
        type_name: &'static str,
        default: Value,
        metavar: Option<&str>,
        help: Option<&str>,
    ) {
        self.actions.push(Action {
            option_strings: vec![option.into()],
            dest: option.trim_start_matches('-').replace('-', "_"),
            help: help.map(str::to_string),
            metavar: metavar.map(str::to_string),
            default: Some(default),
            kind: Kind::Store { convert, type_name },
        });
    }

    pub fn flag(&mut self, option: &str, help: Option<&str>) {
        self.actions.push(Action {
            option_strings: vec![option.into()],
            dest: option.trim_start_matches('-').replace('-', "_"),
            help: help.map(str::to_string),
            metavar: None,
            default: Some(Value::Bool(false)),
            kind: Kind::StoreTrue,
        });
    }

    pub fn positional(&mut self, dest: &str, convert: Convert, type_name: &'static str) {
        self.actions.push(Action {
            option_strings: Vec::new(),
            dest: dest.into(),
            help: None,
            metavar: None,
            default: None,
            kind: Kind::Positional { convert, type_name },
        });
    }

    pub fn subparsers(&mut self, dest: &str, choices: Vec<(String, Option<String>, Parser)>) {
        self.actions.push(Action {
            option_strings: Vec::new(),
            dest: dest.into(),
            help: None,
            metavar: None,
            default: None,
            kind: Kind::Subparsers { choices },
        });
    }

    fn option_actions(&self) -> Vec<(&str, usize)> {
        let mut out = Vec::new();
        for (i, action) in self.actions.iter().enumerate() {
            for s in &action.option_strings {
                out.push((s.as_str(), i));
            }
        }
        out
    }

    fn error(&self, message: &str) -> Exit {
        Exit {
            code: 2,
            stdout: String::new(),
            stderr: format!("{}{}: error: {message}\n", self.format_usage(), self.prog),
        }
    }

    /// `parse_args`.
    pub fn parse_args(&self, args: &[String]) -> Result<Namespace, Exit> {
        let mut namespace = Namespace::new();
        let extras = self.parse_known_args(args, &mut namespace)?;
        if !extras.is_empty() {
            return Err(self.error(&format!("unrecognized arguments: {}", extras.join(" "))));
        }
        Ok(namespace)
    }

    /// `(action index, option string, explicit argument)` candidates.
    fn parse_optional(&self, arg: &str) -> Option<Vec<OptionTuple>> {
        if arg.is_empty() || !arg.starts_with('-') {
            return None;
        }
        let options = self.option_actions();
        if let Some(&(s, i)) = options.iter().find(|(s, _)| *s == arg) {
            return Some(vec![(Some(i), s.to_string(), None, false)]);
        }
        if arg.chars().count() == 1 {
            return None;
        }
        if let Some((name, value)) = arg.split_once('=')
            && let Some(&(s, i)) = options.iter().find(|(s, _)| *s == name)
        {
            return Some(vec![(
                Some(i),
                s.to_string(),
                Some(value.to_string()),
                true,
            )]);
        }
        let mut tuples = Vec::new();
        let second_dash = arg.chars().nth(1) == Some('-');
        let (prefix, explicit) = match arg.split_once('=') {
            Some((p, v)) => (p, Some(v.to_string())),
            None => (arg, None),
        };
        if second_dash {
            for &(s, i) in &options {
                if s.starts_with(prefix) {
                    tuples.push((Some(i), s.to_string(), explicit.clone(), explicit.is_some()));
                }
            }
        } else {
            let short: String = arg.chars().take(2).collect();
            let short_explicit: String = arg.chars().skip(2).collect();
            for &(s, i) in &options {
                if s == short {
                    tuples.push((Some(i), s.to_string(), Some(short_explicit.clone()), false));
                } else if s.starts_with(prefix) {
                    tuples.push((Some(i), s.to_string(), explicit.clone(), explicit.is_some()));
                }
            }
        }
        if !tuples.is_empty() {
            return Some(tuples);
        }
        let mut chars = arg.chars().skip(1);
        let negative = match chars.next() {
            Some('.') => chars.next().is_some_and(|c| c.is_ascii_digit()),
            Some(c) => c.is_ascii_digit(),
            None => false,
        };
        if negative || arg.contains(' ') {
            return None;
        }
        Some(vec![(None, arg.to_string(), None, false)])
    }

    fn parse_known_args(
        &self,
        args: &[String],
        namespace: &mut Namespace,
    ) -> Result<Vec<String>, Exit> {
        for action in &self.actions {
            if let Some(default) = &action.default {
                namespace
                    .entry(action.dest.clone())
                    .or_insert_with(|| default.clone());
            }
        }
        let mut pattern = String::new();
        let mut option_indices: HashMap<usize, Vec<OptionTuple>> = HashMap::new();
        let mut after_dashes = false;
        for (i, arg) in args.iter().enumerate() {
            if after_dashes {
                pattern.push('A');
            } else if arg == "--" {
                pattern.push('-');
                after_dashes = true;
            } else if let Some(tuples) = self.parse_optional(arg) {
                option_indices.insert(i, tuples);
                pattern.push('O');
            } else {
                pattern.push('A');
            }
        }
        let pattern: Vec<char> = pattern.chars().collect();
        let mut seen: Vec<usize> = Vec::new();
        let mut extras: Vec<String> = Vec::new();
        let mut sub_extras: Vec<String> = Vec::new();
        let mut positionals: Vec<usize> = (0..self.actions.len())
            .filter(|&i| self.actions[i].option_strings.is_empty())
            .collect();
        let max_option = option_indices.keys().max().copied();
        let mut start = 0usize;

        macro_rules! fail {
            ($action:expr, $msg:expr) => {{
                let msg: String = $msg;
                let text = match $action {
                    Some(i) => format!("argument {}: {msg}", self.action_name(i)),
                    None => msg,
                };
                return Err(self.error(&text));
            }};
        }

        let mut consume_positionals = |start: usize,
                                       seen: &mut Vec<usize>,
                                       namespace: &mut Namespace,
                                       sub_extras: &mut Vec<String>|
         -> Result<usize, Exit> {
            let rest: String = pattern[start..].iter().collect();
            let Some(&action_index) = positionals.first() else {
                return Ok(start);
            };
            let action = &self.actions[action_index];
            let count = match &action.kind {
                Kind::Subparsers { .. } => {
                    // (-*A[-AO]*)
                    let dashes = rest.chars().take_while(|&c| c == '-').count();
                    if rest[dashes..].starts_with('A') {
                        rest.len()
                    } else {
                        return Ok(start);
                    }
                }
                _ => {
                    // (-*A-*)
                    let dashes = rest.chars().take_while(|&c| c == '-').count();
                    if !rest[dashes..].starts_with('A') {
                        return Ok(start);
                    }
                    let trailing = rest[dashes + 1..].chars().take_while(|&c| c == '-').count();
                    dashes + 1 + trailing
                }
            };
            let mut values: Vec<String> = args[start..start + count].to_vec();
            let is_parser = matches!(action.kind, Kind::Subparsers { .. });
            if is_parser {
                if pattern[start] == '-' {
                    values.remove(0);
                }
            } else if pattern[start..start + count].contains(&'-')
                && let Some(pos) = values.iter().position(|v| v == "--")
            {
                values.remove(pos);
            }
            positionals.remove(0);
            seen.push(action_index);
            match &action.kind {
                Kind::Positional { convert, type_name } => {
                    let value = self.convert(action_index, *convert, type_name, &values[0])?;
                    namespace.insert(action.dest.clone(), value);
                }
                Kind::Subparsers { choices } => {
                    let name = &values[0];
                    let Some((_, _, parser)) = choices.iter().find(|(n, _, _)| n == name) else {
                        let listed: Vec<String> =
                            choices.iter().map(|(n, _, _)| pystr::repr(n)).collect();
                        return Err(self.error(&format!(
                            "argument {}: invalid choice: {} (choose from {})",
                            self.action_name(action_index),
                            pystr::repr(name),
                            listed.join(", ")
                        )));
                    };
                    namespace.insert(action.dest.clone(), Value::Str(name.clone()));
                    let mut sub_namespace = Namespace::new();
                    let rest = parser.parse_known_args(&values[1..], &mut sub_namespace)?;
                    namespace.extend(sub_namespace);
                    sub_extras.extend(rest);
                }
                _ => {}
            }
            Ok(start + count)
        };

        if let Some(max_option) = max_option {
            while start <= max_option {
                let mut next = start;
                while next <= max_option && !option_indices.contains_key(&next) {
                    next += 1;
                }
                if start != next {
                    let end = consume_positionals(start, &mut seen, namespace, &mut sub_extras)?;
                    if end > start {
                        start = end;
                        continue;
                    }
                    start = end;
                }
                if !option_indices.contains_key(&start) {
                    extras.extend(args[start..next].iter().cloned());
                    start = next;
                }
                if start > max_option {
                    break;
                }
                // consume_optional
                let tuples = &option_indices[&start];
                if tuples.len() > 1 {
                    let names: Vec<&str> = tuples.iter().map(|t| t.1.as_str()).collect();
                    fail!(
                        None::<usize>,
                        format!(
                            "ambiguous option: {} could match {}",
                            args[start],
                            names.join(", ")
                        )
                    );
                }
                let (action_index, option_string, explicit, has_sep) = tuples[0].clone();
                let Some(action_index) = action_index else {
                    extras.push(args[start].clone());
                    start += 1;
                    continue;
                };
                let action = &self.actions[action_index];
                let takes_value = matches!(action.kind, Kind::Store { .. });
                let (value, stop) = match explicit {
                    Some(explicit) => {
                        if !takes_value {
                            let single_dash = option_string.chars().nth(1) != Some('-');
                            if single_dash
                                && !explicit.is_empty()
                                && !(has_sep || explicit.starts_with('-'))
                            {
                                // -hx: x would be another flag; none exist here.
                                let leftover: String = format!("-{explicit}");
                                let char0 = option_string.chars().next().unwrap_or('-');
                                let next_option =
                                    format!("{char0}{}", explicit.chars().next().unwrap_or(' '));
                                if self.option_actions().iter().all(|(s, _)| *s != next_option) {
                                    // Run the flag, then keep the rest as an extra.
                                    seen.push(action_index);
                                    if matches!(action.kind, Kind::Help) {
                                        return Err(Exit {
                                            code: 0,
                                            stdout: self.format_help(),
                                            stderr: String::new(),
                                        });
                                    }
                                    namespace.insert(action.dest.clone(), Value::Bool(true));
                                    extras.push(leftover);
                                    start += 1;
                                    continue;
                                }
                            }
                            fail!(
                                Some(action_index),
                                format!("ignored explicit argument {}", pystr::repr(&explicit))
                            );
                        }
                        (Some(explicit), start + 1)
                    }
                    None => {
                        if takes_value {
                            if pattern.get(start + 1) != Some(&'A') {
                                fail!(Some(action_index), "expected one argument".to_string());
                            }
                            (Some(args[start + 1].clone()), start + 2)
                        } else {
                            (None, start + 1)
                        }
                    }
                };
                seen.push(action_index);
                match &action.kind {
                    Kind::Help => {
                        return Err(Exit {
                            code: 0,
                            stdout: self.format_help(),
                            stderr: String::new(),
                        });
                    }
                    Kind::StoreTrue => {
                        namespace.insert(action.dest.clone(), Value::Bool(true));
                    }
                    Kind::Store { convert, type_name } => {
                        let converted = self.convert(
                            action_index,
                            *convert,
                            type_name,
                            &value.unwrap_or_default(),
                        )?;
                        namespace.insert(action.dest.clone(), converted);
                    }
                    _ => {}
                }
                start = stop;
            }
        }
        let stop = consume_positionals(start, &mut seen, namespace, &mut sub_extras)?;
        extras.extend(args[stop.min(args.len())..].iter().cloned());
        let required: Vec<String> = (0..self.actions.len())
            .filter(|&i| self.actions[i].option_strings.is_empty() && !seen.contains(&i))
            .map(|i| self.action_name(i))
            .collect();
        if !required.is_empty() {
            return Err(self.error(&format!(
                "the following arguments are required: {}",
                required.join(", ")
            )));
        }
        extras.extend(sub_extras);
        Ok(extras)
    }

    fn convert(
        &self,
        index: usize,
        convert: Convert,
        type_name: &str,
        text: &str,
    ) -> Result<Value, Exit> {
        convert(text).map_err(|message| {
            let message = message
                .unwrap_or_else(|| format!("invalid {type_name} value: {}", pystr::repr(text)));
            self.error(&format!("argument {}: {message}", self.action_name(index)))
        })
    }

    fn action_name(&self, index: usize) -> String {
        let action = &self.actions[index];
        if !action.option_strings.is_empty() {
            action.option_strings.join("/")
        } else if let Some(metavar) = &action.metavar {
            metavar.clone()
        } else {
            action.dest.clone()
        }
    }

    // --- Help formatting (argparse.HelpFormatter) ---

    fn metavar(&self, action: &Action) -> String {
        if let Some(m) = &action.metavar {
            return m.clone();
        }
        match &action.kind {
            Kind::Subparsers { choices } => {
                format!(
                    "{{{}}}",
                    choices
                        .iter()
                        .map(|(n, _, _)| n.as_str())
                        .collect::<Vec<_>>()
                        .join(",")
                )
            }
            Kind::Positional { .. } => action.dest.clone(),
            _ => action.dest.to_uppercase(),
        }
    }

    fn usage_part(&self, action: &Action) -> String {
        match &action.kind {
            Kind::Subparsers { .. } => format!("{} ...", self.metavar(action)),
            Kind::Positional { .. } => self.metavar(action),
            Kind::Help | Kind::StoreTrue => format!("[{}]", action.option_strings[0]),
            Kind::Store { .. } => {
                format!("[{} {}]", action.option_strings[0], self.metavar(action))
            }
        }
    }

    fn invocation(&self, action: &Action) -> String {
        match &action.kind {
            Kind::Help | Kind::StoreTrue => action.option_strings.join(", "),
            Kind::Store { .. } => action
                .option_strings
                .iter()
                .map(|s| format!("{s} {}", self.metavar(action)))
                .collect::<Vec<_>>()
                .join(", "),
            _ => self.metavar(action),
        }
    }

    fn format_usage_text(&self, width: usize) -> String {
        let prefix = "usage: ";
        let prog = self.prog.clone();
        let opt_parts: Vec<String> = self
            .actions
            .iter()
            .filter(|a| !a.option_strings.is_empty())
            .map(|a| self.usage_part(a))
            .collect();
        let pos_parts: Vec<String> = self
            .actions
            .iter()
            .filter(|a| a.option_strings.is_empty())
            .map(|a| self.usage_part(a))
            .collect();
        let all: Vec<String> = opt_parts.iter().chain(pos_parts.iter()).cloned().collect();
        let action_usage = all.join(" ");
        let usage = [prog.as_str(), action_usage.as_str()]
            .iter()
            .filter(|s| !s.is_empty())
            .copied()
            .collect::<Vec<_>>()
            .join(" ");
        let text_width = width;
        let len = |s: &str| s.chars().count();
        if len(prefix) + len(&usage) <= text_width {
            return format!("{prefix}{usage}\n\n");
        }
        let get_lines = |parts: &[String], indent: &str, with_prefix: bool| -> Vec<String> {
            let mut lines = Vec::new();
            let mut line: Vec<&str> = Vec::new();
            let indent_len = len(indent);
            let mut line_len = if with_prefix {
                len(prefix) - 1
            } else {
                indent_len - 1
            };
            for part in parts {
                if line_len + 1 + len(part) > text_width && !line.is_empty() {
                    lines.push(format!("{indent}{}", line.join(" ")));
                    line.clear();
                    line_len = indent_len - 1;
                }
                line.push(part);
                line_len += len(part) + 1;
            }
            if !line.is_empty() {
                lines.push(format!("{indent}{}", line.join(" ")));
            }
            if with_prefix && !lines.is_empty() {
                lines[0] = lines[0].chars().skip(indent_len).collect();
            }
            lines
        };
        let lines: Vec<String> = if (len(prefix) + len(&prog)) as f64 <= 0.75 * text_width as f64 {
            let indent = " ".repeat(len(prefix) + len(&prog) + 1);
            if !opt_parts.is_empty() {
                let mut first: Vec<String> = vec![prog.clone()];
                first.extend(opt_parts.iter().cloned());
                let mut lines = get_lines(&first, &indent, true);
                lines.extend(get_lines(&pos_parts, &indent, false));
                lines
            } else if !pos_parts.is_empty() {
                let mut first: Vec<String> = vec![prog.clone()];
                first.extend(pos_parts.iter().cloned());
                get_lines(&first, &indent, true)
            } else {
                vec![prog.clone()]
            }
        } else {
            let indent = " ".repeat(len(prefix));
            let parts: Vec<String> = opt_parts.iter().chain(pos_parts.iter()).cloned().collect();
            let mut lines = get_lines(&parts, &indent, false);
            if lines.len() > 1 {
                lines = get_lines(&opt_parts, &indent, false);
                lines.extend(get_lines(&pos_parts, &indent, false));
            }
            let mut all = vec![prog.clone()];
            all.extend(lines);
            all
        };
        format!("{prefix}{}\n\n", lines.join("\n"))
    }

    pub fn format_usage(&self) -> String {
        let (width, _) = formatter_widths();
        finish(&self.format_usage_text(width))
    }

    pub fn format_help(&self) -> String {
        let (width, max_help_position) = formatter_widths();
        let mut out = self.format_usage_text(width);
        if let Some(description) = &self.description {
            out.push_str(&fill(description, width.max(11), ""));
            out.push_str("\n\n");
        }
        // action_max_length spans every section: one formatter adds them all.
        let mut max_length = 0usize;
        for action in &self.actions {
            max_length = max_length.max(self.invocation(action).chars().count() + 2);
            if let Kind::Subparsers { choices } = &action.kind {
                for (name, _, _) in choices {
                    max_length = max_length.max(name.chars().count() + 4);
                }
            }
        }
        let sections: [(&str, bool); 2] = [("positional arguments", false), ("options", true)];
        for (heading, optional) in sections {
            let actions: Vec<&Action> = self
                .actions
                .iter()
                .filter(|a| a.option_strings.is_empty() != optional)
                .collect();
            if actions.is_empty() {
                continue;
            }
            let help_position = (max_length + 2).min(max_help_position);
            let mut items = String::new();
            for action in &actions {
                items.push_str(&format_action(
                    &self.invocation(action),
                    action.help.as_deref(),
                    2,
                    help_position,
                    width,
                ));
                if let Kind::Subparsers { choices } = &action.kind {
                    for (name, help, _) in choices {
                        items.push_str(&format_action(
                            name,
                            help.as_deref(),
                            4,
                            help_position,
                            width,
                        ));
                    }
                }
            }
            out.push_str(&format!("\n{heading}:\n{items}\n"));
        }
        finish(&out)
    }
}

fn finish(text: &str) -> String {
    let mut collapsed = String::new();
    let mut newlines = 0;
    for c in text.chars() {
        if c == '\n' {
            newlines += 1;
            if newlines <= 2 {
                collapsed.push(c);
            }
        } else {
            newlines = 0;
            collapsed.push(c);
        }
    }
    format!("{}\n", collapsed.trim_matches('\n'))
}

fn format_action(
    invocation: &str,
    help: Option<&str>,
    indent: usize,
    help_position: usize,
    width: usize,
) -> String {
    let help_width = width.saturating_sub(help_position).max(11);
    let action_width = help_position.saturating_sub(indent + 2);
    let pad = " ".repeat(indent);
    let help = help.filter(|h| !h.trim().is_empty());
    let Some(help) = help else {
        return format!("{pad}{invocation}\n");
    };
    let (mut out, indent_first) = if invocation.chars().count() <= action_width {
        (format!("{pad}{invocation:<action_width$}  "), 0)
    } else {
        (format!("{pad}{invocation}\n"), help_position)
    };
    let lines = wrap(&normalise_ws(help), help_width);
    if let Some(first) = lines.first() {
        out.push_str(&format!("{}{first}\n", " ".repeat(indent_first)));
        for line in &lines[1..] {
            out.push_str(&format!("{}{line}\n", " ".repeat(help_position)));
        }
    }
    out
}

/// HelpFormatter width and max_help_position, from the terminal size.
fn formatter_widths() -> (usize, usize) {
    let columns = std::env::var("COLUMNS")
        .ok()
        .and_then(|c| pystr::parse_int(&c))
        .filter(|&c| c > 0)
        .map(|c| c as usize)
        .or_else(|| terminal_size::terminal_size_of(std::io::stdout()).map(|(w, _)| w.0 as usize))
        .unwrap_or(80);
    let width = columns.saturating_sub(2);
    let max_help_position = 24usize.min((width as i64 - 20).max(4) as usize);
    (width, max_help_position)
}

fn normalise_ws(text: &str) -> String {
    text.split(pystr::is_space)
        .filter(|w| !w.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

fn fill(text: &str, width: usize, indent: &str) -> String {
    wrap(&normalise_ws(text), width)
        .iter()
        .map(|line| format!("{indent}{line}"))
        .collect::<Vec<_>>()
        .join("\n")
}

static WORDSEP: LazyLock<Regex> = LazyLock::new(|| {
    let ws = r"[\t\n\x0b\x0c\r ]";
    let nws = r"[^\t\n\x0b\x0c\r ]";
    let wp = r#"[\w!"'&.,?]"#;
    let lt = r"[^\d\W]";
    Regex::new(&format!(
        r"({ws}+|(?<={wp})-{{2,}}(?=\w)|{nws}+?(?:-(?:(?<={lt}{{2}}-)|(?<={lt}-{lt}-))(?={lt}-?{lt})|(?={ws}|\z)|(?<={wp})(?=-{{2,}}\w)))"
    ))
    .expect("wordsep compiles")
});

/// `textwrap.wrap(text, width)` with its default settings.
fn wrap(text: &str, width: usize) -> Vec<String> {
    let mut chunks: Vec<String> = Vec::new();
    let mut last = 0;
    for m in WORDSEP.find_iter(text).flatten() {
        if m.start() > last {
            chunks.push(text[last..m.start()].to_string());
        }
        if !m.as_str().is_empty() {
            chunks.push(m.as_str().to_string());
        }
        last = m.end();
    }
    if last < text.len() {
        chunks.push(text[last..].to_string());
    }
    chunks.reverse();
    let mut lines: Vec<String> = Vec::new();
    let len = |s: &str| s.chars().count();
    while !chunks.is_empty() {
        let mut line: Vec<String> = Vec::new();
        let mut cur = 0;
        if chunks.last().is_some_and(|c| c.trim().is_empty()) && !lines.is_empty() {
            chunks.pop();
        }
        while let Some(chunk) = chunks.last() {
            let l = len(chunk);
            if cur + l <= width {
                cur += l;
                line.push(chunks.pop().unwrap_or_default());
            } else {
                break;
            }
        }
        if let Some(chunk) = chunks.last().cloned()
            && len(&chunk) > width
        {
            let space_left = if width > cur { width - cur } else { 1 };
            let chars: Vec<char> = chunk.chars().collect();
            let mut end = space_left;
            if chars.len() > space_left {
                let head: String = chars[..space_left].iter().collect();
                if let Some(hyphen) = head.rfind('-')
                    && hyphen > 0
                    && chars[..hyphen].iter().any(|&c| c != '-')
                {
                    end = hyphen + 1;
                }
            }
            line.push(chars[..end.min(chars.len())].iter().collect());
            let rest: String = chars[end.min(chars.len())..].iter().collect();
            chunks.pop();
            chunks.push(rest);
        }
        if line.last().is_some_and(|c| c.trim().is_empty()) {
            line.pop();
        }
        if !line.is_empty() {
            lines.push(line.concat());
        }
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wraps_like_textwrap() {
        assert_eq!(
            wrap(
                "maximum extra unchanged-detail requests across this run",
                54
            ),
            ["maximum extra unchanged-detail requests across this", "run"]
        );
        assert_eq!(
            wrap(
                "maximum extra unchanged-detail requests across this run",
                20
            ),
            [
                "maximum extra",
                "unchanged-detail",
                "requests across this",
                "run"
            ]
        );
    }
}
