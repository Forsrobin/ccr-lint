//! Command line: `ccr-lint lint` (commitlint) and `ccr-lint release` (commit-and-tag-version).

use std::io::{IsTerminal, Read};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::{Args, Parser, Subcommand};

use crate::color::Palette;
use crate::config::Config;
use crate::git;
use crate::js;
use crate::lint::{self, FormatOptions, LintOptions};
use crate::release::{self, ReleaseArgs};

#[derive(Parser)]
#[command(
    name = "ccr-lint",
    version,
    about = "Conventional Commits linting and releases, without Node"
)]
struct Cli {
    /// Configuration file (default: ccr-lint.toml, then Cargo.toml metadata).
    #[arg(long, short = 'g', global = true)]
    config: Option<PathBuf>,
    #[command(subcommand)]
    command: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Lint commit messages like commitlint (reads stdin without --edit/--env/--from/--to/--last).
    Lint(LintArgs),
    /// Bump the version, write the changelog, commit and tag, like commit-and-tag-version.
    Release(ReleaseCli),
}

#[derive(Args)]
struct LintArgs {
    /// Lint the message in this file (default: .git/COMMIT_EDITMSG).
    #[arg(long, short = 'e', num_args = 0..=1, default_missing_value = "")]
    edit: Option<String>,
    /// Lint the message in the file named by this environment variable.
    #[arg(long, short = 'E')]
    env: Option<String>,
    /// Lower end of the commit range to lint.
    #[arg(long, short = 'f')]
    from: Option<String>,
    /// Upper end of the commit range to lint.
    #[arg(long, short = 't')]
    to: Option<String>,
    /// Use the last tag as the lower end of the range.
    #[arg(long)]
    from_last_tag: bool,
    /// Lint only the last commit.
    #[arg(long, short = 'l')]
    last: bool,
    /// Show the report for valid messages too.
    #[arg(long, short = 'V')]
    verbose: bool,
    /// Print nothing.
    #[arg(long, short = 'q')]
    quiet: bool,
    /// Exit 2 on warnings and 3 on errors.
    #[arg(long, short = 's')]
    strict: bool,
    /// Help url printed with problems.
    #[arg(long, short = 'H')]
    help_url: Option<String>,
    /// Force colors on.
    #[arg(long, overrides_with = "no_color")]
    color: bool,
    /// Turn colors off.
    #[arg(long)]
    no_color: bool,
    /// Directory to run in.
    #[arg(long, short = 'd')]
    cwd: Option<PathBuf>,
    /// Message to lint, instead of stdin.
    message: Vec<String>,
}

#[derive(Args)]
struct ReleaseCli {
    /// Show what would happen without changing anything.
    #[arg(long)]
    dry_run: bool,
    /// major, minor, patch or an exact version.
    #[arg(long, short = 'r')]
    release_as: Option<String>,
    /// Do not bump the version (first release).
    #[arg(long, short = 'f')]
    first_release: bool,
    #[arg(long)]
    skip_bump: bool,
    #[arg(long)]
    skip_changelog: bool,
    #[arg(long)]
    skip_commit: bool,
    #[arg(long)]
    skip_tag: bool,
    /// Print nothing.
    #[arg(long)]
    silent: bool,
    /// Override the tag prefix.
    #[arg(long, short = 't')]
    tag_prefix: Option<String>,
    /// Pass --no-verify to git commit.
    #[arg(long, short = 'n')]
    no_verify: bool,
    /// Sign the commit and tag.
    #[arg(long, short = 's')]
    sign: bool,
    /// Accepted for compatibility; colors follow the environment.
    #[arg(long, hide = true)]
    no_color: bool,
}

pub fn main() -> ExitCode {
    let cli = Cli::parse();
    let cwd = match &cli.command {
        Cmd::Lint(a) => a.cwd.clone(),
        Cmd::Release(_) => None,
    }
    .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));
    let root = toplevel(&cwd).unwrap_or_else(|| cwd.clone());
    let config = match Config::load(&root, cli.config.as_deref()) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("ccr-lint: {e}");
            return ExitCode::from(9);
        }
    };
    match cli.command {
        Cmd::Lint(a) => run_lint(a, &cwd, &config),
        Cmd::Release(a) => {
            let mut cfg = config.release.clone();
            if let Some(p) = a.tag_prefix {
                cfg.tag_prefix = p;
            }
            cfg.no_verify |= a.no_verify;
            cfg.sign |= a.sign;
            let args = ReleaseArgs {
                dry_run: a.dry_run,
                release_as: a.release_as,
                first_release: a.first_release,
                skip_bump: a.skip_bump,
                skip_changelog: a.skip_changelog,
                skip_commit: a.skip_commit,
                skip_tag: a.skip_tag,
                silent: a.silent,
            };
            match release::run(&cwd, &cfg, &args) {
                Ok(()) => ExitCode::SUCCESS,
                Err(_) => ExitCode::FAILURE,
            }
        }
    }
}

/// `@commitlint/top-level`: the nearest directory containing `.git`.
fn toplevel(cwd: &Path) -> Option<PathBuf> {
    cwd.ancestors().find(|d| d.join(".git").exists()).map(Path::to_path_buf)
}

fn fail(msg: &str) -> ExitCode {
    eprintln!("{msg}");
    ExitCode::FAILURE
}

fn edit_file_path(top: &Path, edit: &str) -> Result<PathBuf, String> {
    if !edit.is_empty() {
        return Ok(top.join(edit));
    }
    let dotgit = top.join(".git");
    if dotgit.is_dir() {
        return Ok(dotgit.join("COMMIT_EDITMSG"));
    }
    let text = std::fs::read_to_string(&dotgit).map_err(|e| e.to_string())?;
    let rel = text.replacen("gitdir: ", "", 1).replacen('\n', "", 1);
    Ok(top.join(rel).join("COMMIT_EDITMSG"))
}

fn read_messages(a: &LintArgs, cwd: &Path) -> Result<Vec<String>, String> {
    let edit = match &a.env {
        Some(var) => Some(std::env::var(var).map_err(|_| {
            format!(
                "Received '{var}' as value for -E | --env, but environment variable '{var}' is not available globally"
            )
        })?),
        None => a.edit.clone(),
    };
    if let Some(edit) = edit {
        let top = toplevel(cwd).ok_or_else(|| format!("Could not find git root from {}", cwd.display()))?;
        let path = edit_file_path(&top, &edit)?;
        let bytes = std::fs::read(&path).map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                let hint = if edit.is_empty() {
                    "--edit reads the message prepared by 'git commit' and is intended to run from a commit-msg hook. If you want to lint existing history, use --from / --to instead; to lint a specific file, pass its path as --edit <file>."
                } else {
                    "Check that the path passed to --edit exists and is readable."
                };
                format!("No commit message file found at {}. {hint}", path.display())
            } else {
                e.to_string()
            }
        })?;
        return Ok(vec![format!("{}\n", String::from_utf8_lossy(&bytes))]);
    }
    if a.last {
        let out = git::run(cwd, &["log", "-1", "--pretty=format:%B"]).unwrap_or_default();
        let mut s = js::trim(&out).to_owned();
        if s.len() >= 2 && s.starts_with('"') && s.ends_with('"') {
            s = s[1..s.len() - 1].to_owned();
        }
        return Ok(vec![s]);
    }
    let mut from = a.from.clone();
    if from.is_none() && a.from_last_tag {
        let out = git::run(
            cwd,
            &[
                "describe",
                "--abbrev=40",
                "--always",
                "--first-parent",
                "--long",
                "--tags",
            ],
        )
        .unwrap_or_default();
        let s = js::trim(&out);
        if s.len() == 40 {
            from = Some(s.to_owned());
        } else if s.len() >= 43 {
            let cut = s[..s.len() - 43 + 1].rfind('-').unwrap_or(0);
            from = Some(s[..cut].to_owned());
        }
    }
    if let Some(f) = &from {
        let to = a.to.as_deref().unwrap_or("HEAD");
        let mb = git::output(cwd, &["merge-base", f, to]).map_err(|e| e.0)?;
        if mb.status.code() == Some(1) {
            return Err(format!(
                "Cannot find merge-base between '{f}' and '{to}'. This typically indicates incomplete git history (e.g., a shallow clone). Consider fetching more history."
            ));
        }
    }
    let to = a.to.clone().unwrap_or_else(|| "HEAD".into());
    git::raw_commits(cwd, "%B", from.as_deref(), &to, false).map_err(|e| e.0)
}

fn run_lint(a: LintArgs, cwd: &Path, config: &Config) -> ExitCode {
    let from_history = a.from.is_some() || a.from_last_tag || a.to.is_some() || a.last;
    let from_edit = a.edit.is_some() || a.env.is_some();
    if a.last && (a.from.is_some() || a.to.is_some() || a.edit.is_some()) {
        println!("Please use the --last flag alone. The --last flag should not be used with --to or --from or --edit.");
        return ExitCode::FAILURE;
    }
    if let (Some(f), Some(t)) = (&a.from, &a.to)
        && f == t
    {
        eprintln!(
            "Invalid input flags: --from and --to point to the same commit. To lint that single commit, use --last (if HEAD) or --from {f}^ --to {f}."
        );
        return ExitCode::from(9);
    }
    let input = if !a.message.is_empty() {
        Ok(vec![a.message.join(" ")])
    } else if from_history || from_edit {
        read_messages(&a, cwd)
    } else {
        let mut s = String::new();
        if !std::io::stdin().is_terminal() {
            let _ = std::io::stdin().read_to_string(&mut s);
        }
        Ok(vec![s])
    };
    let messages: Vec<String> = match input {
        Ok(m) => m.into_iter().filter(|m| !js::trim(m).is_empty()).collect(),
        Err(e) => return fail(&format!("Error: {e}")),
    };
    if messages.is_empty() && !(from_history || from_edit) {
        println!("[input] is required: supply via stdin, or --env or --edit or --last or --from and --to");
        return ExitCode::FAILURE;
    }

    let mut opts = LintOptions {
        default_ignores: config.lint.default_ignores,
        ignores: config.lint.ignores.iter().map(|p| js::re(p)).collect(),
        help_url: config.lint.help_url.clone(),
        comment_char: None,
    };
    if from_edit {
        let out = std::process::Command::new("git")
            .args(["config", "core.commentChar"])
            .output();
        opts.comment_char = Some(match out {
            Ok(o) if o.status.code().is_some_and(|c| c > 1) => {
                eprintln!(
                    "Could not determine core.commentChar git configuration {}",
                    String::from_utf8_lossy(&o.stderr)
                );
                "#".to_owned()
            }
            Ok(o) => {
                let s = js::trim(&String::from_utf8_lossy(&o.stdout)).to_owned();
                if s.is_empty() { "#".to_owned() } else { s }
            }
            Err(_) => "#".to_owned(),
        });
    }

    let mut results = Vec::new();
    for m in &messages {
        match lint::lint(m, &config.lint.rules, &opts) {
            Ok(r) => results.push(r),
            Err(e) => return fail(&e),
        }
    }
    let colors = if a.no_color {
        Palette::off()
    } else {
        Palette::picocolors()
    };
    let help = a
        .help_url
        .as_deref()
        .map(js::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or(&config.lint.help_url)
        .to_owned();
    let output = lint::format(
        &results,
        &FormatOptions {
            verbose: a.verbose,
            help_url: Some(&help),
            colors,
        },
    );
    if !a.quiet && !output.is_empty() {
        println!("{output}");
    }
    let errors: usize = results.iter().map(|r| r.errors.len()).sum();
    let warnings: usize = results.iter().map(|r| r.warnings.len()).sum();
    if a.strict {
        if errors > 0 {
            return ExitCode::from(3);
        }
        if warnings > 0 {
            return ExitCode::from(2);
        }
    }
    if results.iter().any(|r| !r.valid) {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    }
}
