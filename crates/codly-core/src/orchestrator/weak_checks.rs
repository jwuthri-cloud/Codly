//! Does a verify command prove anything about the work? Pure: no I/O.
//!
//! The gate believes a command's exit code. A command that exits 0 whatever the worker did
//! proves nothing: `true`, `echo ok`, `cargo test || true`, `pnpm test | tail -20` (the shell
//! reports the exit code of `tail`), `pnpm test; echo done`. A command that only looks for a
//! file (`test -f src/app.ts`) proves that the file is there, not that it is right. The
//! planner marks a task whose every verify command is like that (`planner::validate`), and the
//! plan card shows the mark before you run the plan.
//!
//! Only shapes that are certain are judged. What this does not read (a subshell, an `if`, a
//! command substitution, a here-document, a program it does not know) counts as a real check.

/// What the exit code of a command says about the work. Ordered: a later one says more.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Proof {
    /// It exits 0 whatever the work is.
    Nothing,
    /// It fails only when a file or a folder is missing.
    FileExists,
    /// It can fail on the work, or this cannot tell.
    Real,
}

/// The operator before a simple command.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Op {
    /// `;`, a new line, or the start of the text.
    Seq,
    And,
    Or,
    Pipe,
}

/// Programs that exit 0 whatever happened before them.
const NO_OPS: &[&str] = &["true", ":", "echo", "printf", "pwd", "sleep", "set", "export"];
/// Programs that fail only when a path is missing.
const PATH_ONLY: &[&str] = &["cd", "ls"];
/// At the end of a pipe, these pass the output on and exit 0: the exit code of the command
/// before them is lost (without `set -o pipefail`).
const FILTERS: &[&str] = &["tail", "head", "tee", "cat", "wc", "sort", "uniq"];
/// The start of a compound command: not read.
const KEYWORDS: &[&str] = &["{", "}", "!", "if", "then", "else", "elif", "fi", "for", "while", "until", "do", "done", "case", "esac", "function", "select", "time"];
/// `test` operators that ask whether a path is there.
const PATH_TESTS: &[&str] = &["-e", "-f", "-d", "-s", "-r", "-w", "-x", "-L", "-h"];

/// Why a verify command proves nothing about the work, in words for the plan card. None = it
/// can fail on the work (or its shape is not one this reads).
pub fn weakness(cmd: &str) -> Option<&'static str> {
    match judge(cmd) {
        Proof::Nothing => Some("always passes"),
        Proof::FileExists => Some("only checks that a file is there"),
        Proof::Real => None,
    }
}

/// The simple commands of `cmd` as words, each with the operator before it. None = a shape
/// this does not read.
fn split(cmd: &str) -> Option<Vec<(Op, Vec<String>)>> {
    let mut out: Vec<(Op, Vec<String>)> = vec![];
    let mut op = Op::Seq;
    let mut words: Vec<String> = vec![];
    // The word being read; None between words (`""` is a word, an empty one).
    let mut word: Option<String> = None;
    let mut prev = ' ';
    let mut chars = cmd.chars().peekable();
    while let Some(c) = chars.next() {
        let mut next_op = None;
        match c {
            '\\' => match chars.next() {
                Some('\n') | None => {}
                Some(x) => word.get_or_insert_default().push(x),
            },
            '\'' => {
                let w = word.get_or_insert_default();
                loop {
                    match chars.next()? {
                        '\'' => break,
                        x => w.push(x),
                    }
                }
            }
            '"' => {
                let w = word.get_or_insert_default();
                loop {
                    match chars.next()? {
                        '"' => break,
                        '`' => return None,
                        '$' if chars.peek() == Some(&'(') => return None,
                        '\\' => w.push(chars.next()?),
                        x => w.push(x),
                    }
                }
            }
            '`' | '(' | ')' => return None,
            '$' if chars.peek() == Some(&'(') => return None,
            '<' if chars.peek() == Some(&'<') => return None,
            '#' if word.is_none() => {
                while chars.peek().is_some_and(|x| *x != '\n') {
                    chars.next();
                }
            }
            ' ' | '\t' | '\r' => words.extend(word.take()),
            '\n' | ';' => {
                words.extend(word.take());
                // A new line after an operator, or after a new line, joins nothing.
                if c == ';' || !words.is_empty() {
                    next_op = Some(Op::Seq);
                }
            }
            // `>&2`, `&>file`: part of a redirection. A lone `&` (a background job) is not read.
            '&' if chars.peek() == Some(&'&') => {
                chars.next();
                next_op = Some(Op::And);
            }
            '&' if prev == '>' || chars.peek() == Some(&'>') => word.get_or_insert_default().push(c),
            '&' => return None,
            '|' if chars.peek() == Some(&'|') => {
                chars.next();
                next_op = Some(Op::Or);
            }
            // `>|file`: a redirection.
            '|' if prev == '>' => word.get_or_insert_default().push(c),
            '|' => {
                // `|&` pipes the errors too.
                if chars.peek() == Some(&'&') {
                    chars.next();
                }
                next_op = Some(Op::Pipe);
            }
            x => word.get_or_insert_default().push(x),
        }
        prev = c;
        if let Some(next) = next_op {
            words.extend(word.take());
            if words.is_empty() {
                return None; // an operator with no command before it
            }
            out.push((op, std::mem::take(&mut words)));
            op = next;
        }
    }
    words.extend(word.take());
    if !words.is_empty() {
        out.push((op, words));
    }
    Some(out)
}

/// The program a simple command starts: its first word that is not `NAME=value`, without its
/// folder. None = only assignments.
fn program(words: &[String]) -> Option<(&str, &[String])> {
    let assignment = |w: &String| {
        w.split_once('=').is_some_and(|(name, _)| {
            !name.is_empty() && !name.starts_with(|c: char| c.is_ascii_digit()) && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
        })
    };
    let at = words.iter().position(|w| !assignment(w))?;
    let name = words[at].rsplit('/').next().unwrap_or(&words[at]);
    Some((name, &words[at + 1..]))
}

/// Do the arguments of `test` (or `[`) only ask whether paths are there?
fn path_test(args: &[String]) -> bool {
    let args = match args.split_last() {
        Some((last, rest)) if last == "]" => rest,
        _ => args,
    };
    let operator = |a: &&String| a.starts_with('-') || a.contains('=');
    args.iter().any(|a| PATH_TESTS.contains(&a.as_str())) && args.iter().filter(operator).all(|a| PATH_TESTS.contains(&a.as_str()) || a == "-a" || a == "-o")
}

fn simple(words: &[String]) -> Proof {
    let Some((name, args)) = program(words) else { return Proof::Nothing };
    match name {
        "exit" if args.first().is_none_or(|code| code == "0") => Proof::Nothing,
        "test" | "[" if path_test(args) => Proof::FileExists,
        _ if NO_OPS.contains(&name) => Proof::Nothing,
        _ if PATH_ONLY.contains(&name) => Proof::FileExists,
        _ => Proof::Real,
    }
}

/// `set -e` (or `set -o errexit`) and `set -o pipefail`, anywhere in the text.
fn shell_options(cmds: &[(Op, Vec<String>)]) -> (bool, bool) {
    let sets: Vec<&[String]> = cmds.iter().filter_map(|(_, words)| program(words).filter(|(name, _)| *name == "set").map(|(_, args)| args)).collect();
    let flag = |a: &String| a.starts_with('-') && !a.starts_with("--") && a.contains('e');
    let errexit = sets.iter().any(|args| args.iter().any(|a| flag(a) || a == "errexit"));
    let pipefail = sets.iter().any(|args| args.iter().any(|a| a == "pipefail"));
    (errexit, pipefail)
}

fn judge(cmd: &str) -> Proof {
    let Some(cmds) = split(cmd) else { return Proof::Real };
    if cmds.is_empty() || cmds.iter().any(|(_, words)| program(words).is_some_and(|(name, _)| KEYWORDS.contains(&name))) {
        return Proof::Real;
    }
    let (errexit, pipefail) = shell_options(&cmds);
    // One entry for each `;`-separated list: `a && b | c || d` is one.
    let mut lists: Vec<Proof> = vec![];
    let mut at = 0;
    while at < cmds.len() {
        let join = cmds[at].0;
        let end = at + 1 + cmds[at + 1..].iter().take_while(|(op, _)| *op == Op::Pipe).count();
        let (last, before) = cmds[at..end].split_last().expect("a pipeline has a command");
        // The shell reports the last command of a pipe; with pipefail, any that failed.
        let last = match program(&last.1) {
            Some((name, _)) if !before.is_empty() && FILTERS.contains(&name) => Proof::Nothing,
            _ => simple(&last.1),
        };
        let pipeline = if pipefail { before.iter().map(|(_, words)| simple(words)).fold(last, Proof::max) } else { last };
        // An `exit 0` of its own ends the script: nothing after it runs.
        let ends = join == Op::Seq && end == at + 1 && program(&cmds[at].1).is_some_and(|(name, _)| name == "exit") && pipeline == Proof::Nothing;
        match (join, lists.last_mut()) {
            // `a && b` fails when one of them does.
            (Op::And, Some(list)) => *list = (*list).max(pipeline),
            // `a || b` fails only when both do: never, if one of them cannot fail.
            (Op::Or, Some(list)) => *list = (*list).min(pipeline),
            _ => lists.push(pipeline),
        }
        if ends {
            break;
        }
        at = end;
    }
    // The shell reports the last list; with `set -e`, the first one that failed.
    let all = lists.iter().copied();
    let proof = if errexit { all.max() } else { all.last() };
    proof.unwrap_or(Proof::Real)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn commands_that_cannot_fail_prove_nothing() {
        let always = [
            "true",
            ":",
            "echo ok",
            "/usr/bin/true",
            "printf 'done\\n'",
            "exit 0",
            "exit",
            "CI=1 echo \"all good\"",
            "echo ok > /dev/null 2>&1",
            // The exit code of the test run is thrown away.
            "cargo test || true",
            "pnpm test || echo 'tests failed'",
            "pnpm test; echo done",
            "pnpm test\necho done",
            "npm test || exit 0",
            "cargo test 2>&1 | tail -40",
            "pnpm check |& tee check.log",
            "pnpm test | cat",
            "exit 0; cargo test",
            "true && echo ok",
            "true || cargo test",
            "set -e; cargo test || true; echo ok",
            "pnpm test || true # keep going",
        ];
        for cmd in always {
            assert_eq!(weakness(cmd), Some("always passes"), "{cmd}");
        }
    }

    #[test]
    fn commands_that_only_look_for_a_file_are_marked() {
        let files = [
            "test -f src/app.ts",
            "[ -f src/app.ts ]",
            "test -d dist -a -s dist/index.js",
            "[ ! -e old.txt ]",
            "test -f \"$OUT/report.md\"",
            "ls src/app.ts",
            "cd web",
            "cd web && ls",
            "test -f a.txt && echo found",
            // Passes when the file is there, whatever the tests say.
            "test -f a.txt || cargo test",
            "cargo test | tail -5; test -f a.txt",
        ];
        for cmd in files {
            assert_eq!(weakness(cmd), Some("only checks that a file is there"), "{cmd}");
        }
    }

    #[test]
    fn commands_that_can_fail_on_the_work_are_real_checks() {
        let real = [
            "cargo test -p codly-core --lib planner",
            "pnpm exec vitest run src/lib/__tests__/plan.test.ts",
            "true && cargo test",
            "cd web && pnpm test",
            "cd web; pnpm test",
            "CI=1 pnpm check",
            "cargo test || exit 1",
            "cargo test && echo ok",
            "grep -q hello hello.txt",
            "cargo test 2>&1 | grep -q 'test result: ok'",
            "set -o pipefail; cargo test 2>&1 | tail -40",
            "set -euo pipefail\npnpm test\necho done",
            "test -n \"$CODLY_PORT_BASE\" && grep -q hello hello.txt",
            "test \"$(cat VERSION)\" = 2",
            "[ \"$A\" = b ]",
            "exit 1",
            "false",
            "echo 'a || true' | grep -q true",
            "grep -q 'x; echo' file",
            "pnpm test >&2",
            "pnpm test &> out.log",
            "pnpm test # || true",
            // Shapes this does not read: never marked.
            "echo $(cargo test)",
            "(cd web && pnpm test)",
            "if cargo test; then echo ok; fi",
            "! grep -q TODO src/app.ts",
            "pnpm dev & sleep 2",
            "cat <<EOF\ntrue\nEOF",
            "echo `date`",
            "echo 'unclosed",
            "&& true",
            "",
        ];
        for cmd in real {
            assert_eq!(weakness(cmd), None, "{cmd}");
        }
    }

    #[test]
    fn the_text_is_split_at_operators_outside_quotes() {
        let words = |cmd: &str| split(cmd).unwrap().into_iter().map(|(op, w)| (op, w.join(" "))).collect::<Vec<_>>();
        assert_eq!(
            words("A=1 pnpm test 2>&1 | tail -3 && echo \"a;b\" || 'x|y' ; z\\ w"),
            [(Op::Seq, "A=1 pnpm test 2>&1".to_string()), (Op::Pipe, "tail -3".into()), (Op::And, "echo a;b".into()), (Op::Or, "x|y".into()), (Op::Seq, "z w".into())]
        );
        // A new line after an operator continues the list; a comment ends at the line's end.
        assert_eq!(words("a &&\n  b # c; d\ne"), [(Op::Seq, "a".to_string()), (Op::And, "b".into()), (Op::Seq, "e".into())]);
        assert_eq!(words("echo \"\" x"), [(Op::Seq, "echo  x".to_string())]);
        assert!(split("a; ; b").is_none() && split("a | | b").is_none());
    }
}
