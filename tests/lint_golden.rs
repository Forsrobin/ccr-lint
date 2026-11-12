//! Golden tests: ccr-lint must print exactly what commitlint 21.2.4 printed (recorded by
//! scripts/gen_lint_golden.py) for every message of tests/corpus/lint-cases.txt, via `--edit`.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]

use std::process::Command;

#[test]
fn matches_commitlint_output() {
    let golden: serde_json::Value =
        serde_json::from_str(include_str!("corpus/lint-expected.json")).expect("golden data is valid JSON");
    let dir = tempfile::tempdir().expect("tempdir");
    let init = Command::new("git")
        .args(["init", "-q"])
        .current_dir(dir.path())
        .status()
        .expect("git init");
    assert!(init.success());
    let mut failures = Vec::new();
    for (i, case) in golden.as_array().expect("array").iter().enumerate() {
        let message = case["message"].as_str().expect("message");
        let file = dir.path().join(format!("{i}.msg"));
        std::fs::write(&file, message).expect("write message");
        let out = Command::new(env!("CARGO_BIN_EXE_ccr-lint"))
            .args(["lint", "--edit"])
            .arg(&file)
            .current_dir(dir.path())
            .env("NO_COLOR", "1")
            .env_remove("FORCE_COLOR")
            .env_remove("CI")
            .output()
            .expect("run ccr-lint");
        let stdout = String::from_utf8_lossy(&out.stdout);
        let code = i64::from(out.status.code().unwrap_or(-1));
        if stdout != case["stdout"].as_str().unwrap_or_default() || Some(code) != case["exit"].as_i64() {
            failures.push(format!(
                "{message:?}\n--- commitlint (exit {})\n{}--- ccr-lint (exit {code})\n{stdout}",
                case["exit"],
                case["stdout"].as_str().unwrap_or_default()
            ));
        }
    }
    assert!(
        failures.is_empty(),
        "{} mismatches:\n{}",
        failures.len(),
        failures.join("\n")
    );
}
