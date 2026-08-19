use std::os::unix::fs::PermissionsExt;

use herdr_talon::config::{Config, PatternDefinition};
use herdr_talon::herdr::{Herdr, InvocationContext};
use herdr_talon::snapshot::{launch, launch_with_reporting, LaunchOutcome, RunStore};
use tempfile::tempdir;

fn fake_herdr() -> (tempfile::TempDir, Herdr) {
    let dir = tempdir().unwrap();
    let binary = dir.path().join("herdr");
    std::fs::write(
        &binary,
        r#"#!/bin/sh
printf '%s\n' "$*" >> "$0.log"
if [ "$1 $2" = "pane layout" ]; then
  printf '%s\n' '{"result":{"layout":{"workspace_id":"w1","tab_id":"w1:t1","zoomed":false,"area":{"x":30,"y":1,"width":40,"height":10},"focused_pane_id":"w1:p1","panes":[{"pane_id":"w1:p1","focused":true,"rect":{"x":30,"y":1,"width":20,"height":10}},{"pane_id":"w1:p2","focused":false,"rect":{"x":50,"y":1,"width":20,"height":10}}]},"type":"pane_layout"}}'
elif [ "$1 $2" = "pane read" ]; then
  pane="$3"
  format=text
  source=recent-unwrapped
  while [ "$#" -gt 0 ]; do
    if [ "$1" = "--format" ]; then format="$2"; fi
    if [ "$1" = "--source" ]; then source="$2"; fi
    shift
  done
  if [ "$pane" != "w1:p1" ]; then
    printf '%s\n' 'unexpected pane read' >&2
    exit 3
  elif [ "$source" = "recent-unwrapped" ] && [ -e "$0.empty-recent" ]; then
    :
  elif [ "$source" = "visible" ]; then
    if [ "$format" = "ansi" ]; then printf '\033[36mvisible-value\033[0m\n'; else printf 'visible-value\n'; fi
  else
    if [ "$format" = "ansi" ]; then printf '\033[33mdeadbeef\033[0m\n'; else printf 'deadbeef\n'; fi
  fi
elif [ "$1 $2 $3" = "plugin pane open" ]; then
  : > "$0.pane-open"
  printf '%s\n' '{"id":"cli:plugin","result":{"type":"plugin_pane_opened","plugin_pane":{"plugin_id":"shadowfax.talon","entrypoint":"picker","pane":{"pane_id":"w1:p3"}}}}'
elif [ "$1 $2 $3" = "plugin pane close" ]; then
  if [ -e "$0.pane-open" ]; then
    rm "$0.pane-open"
    printf '%s\n' '{"id":"cli:plugin","result":{"type":"plugin_pane_closed","pane_id":"w1:p3"}}'
  else
    printf '%s\n' '{"id":"cli:plugin","error":{"code":"plugin_pane_not_found","message":"plugin pane not found"}}' >&2
    exit 1
  fi
elif [ "$1 $2" = "notification show" ]; then
  printf '%s\n' '{"result":{"type":"ok"}}'
else
  printf '%s\n' 'unsupported fake command' >&2
  exit 2
fi
"#,
    )
    .unwrap();
    let mut permissions = std::fs::metadata(&binary).unwrap().permissions();
    permissions.set_mode(0o700);
    std::fs::set_permissions(&binary, permissions).unwrap();
    let herdr = Herdr::new(&binary);
    (dir, herdr)
}

#[test]
fn second_launch_closes_the_existing_picker_before_capturing_again() {
    let (fake_dir, herdr) = fake_herdr();
    let state = tempdir().unwrap();
    let store = RunStore::new(state.path()).unwrap();
    let source_context = InvocationContext::parse(
        r#"{"focused_pane_id":"w1:p1","focused_pane_cwd":"/tmp/project"}"#,
    )
    .unwrap();
    let picker_context = InvocationContext::parse(r#"{"focused_pane_id":"w1:p3"}"#).unwrap();

    let first = launch(&herdr, &source_context, &Config::default(), &store).unwrap();
    assert!(matches!(first, LaunchOutcome::Opened { .. }));
    let second = launch(&herdr, &picker_context, &Config::default(), &store).unwrap();
    assert_eq!(
        second,
        LaunchOutcome::Closed {
            pane_id: "w1:p3".into()
        }
    );

    let log = std::fs::read_to_string(fake_dir.path().join("herdr.log")).unwrap();
    assert_eq!(
        log.lines()
            .filter(|line| line.starts_with("plugin pane open "))
            .count(),
        1
    );
    assert_eq!(
        log.lines()
            .filter(|line| line.starts_with("pane layout "))
            .count(),
        1
    );
    assert!(log.lines().any(|line| line == "plugin pane close w1:p3"));
}

#[test]
fn a_stale_picker_record_is_replaced_after_the_pane_has_closed() {
    let (fake_dir, herdr) = fake_herdr();
    let state = tempdir().unwrap();
    let store = RunStore::new(state.path()).unwrap();
    let context = InvocationContext::parse(r#"{"focused_pane_id":"w1:p1"}"#).unwrap();

    let LaunchOutcome::Opened { run_id } =
        launch(&herdr, &context, &Config::default(), &store).unwrap()
    else {
        panic!("expected first overlay launch");
    };
    store.claim(&run_id).unwrap();
    std::fs::remove_file(fake_dir.path().join("herdr.pane-open")).unwrap();

    let second = launch(&herdr, &context, &Config::default(), &store).unwrap();
    assert!(matches!(second, LaunchOutcome::Opened { .. }));

    let log = std::fs::read_to_string(fake_dir.path().join("herdr.log")).unwrap();
    assert_eq!(
        log.lines()
            .filter(|line| line.starts_with("plugin pane open "))
            .count(),
        2
    );
    assert!(log.lines().any(|line| line == "plugin pane close w1:p3"));
}

#[test]
fn launch_captures_only_focused_history_and_opens_a_responsive_popup() {
    let (fake_dir, herdr) = fake_herdr();
    let state = tempdir().unwrap();
    let store = RunStore::new(state.path()).unwrap();
    let context = InvocationContext::parse(
        r#"{"focused_pane_id":"w1:p1","focused_pane_cwd":"/tmp/project"}"#,
    )
    .unwrap();

    let outcome = launch(&herdr, &context, &Config::default(), &store).unwrap();
    let LaunchOutcome::Opened { run_id } = outcome else {
        panic!("expected popup launch");
    };
    let snapshot = store.claim(&run_id).unwrap();
    let log = std::fs::read_to_string(fake_dir.path().join("herdr.log")).unwrap();

    assert_eq!(snapshot.source_pane_id, "w1:p1");
    assert_eq!(snapshot.text, "deadbeef\n");
    assert!(snapshot.ansi.contains("\u{1b}[33m"));
    assert!(!snapshot.history_limited);
    assert_eq!(
        snapshot
            .targets
            .iter()
            .map(|target| target.text.as_str())
            .collect::<Vec<_>>(),
        vec!["deadbeef"]
    );
    assert_eq!(snapshot.alphabet, Config::default().alphabet);
    assert!(log.contains("pane read w1:p1 --source recent-unwrapped --lines 1000 --format text"));
    assert!(log.contains("pane read w1:p1 --source recent-unwrapped --lines 1000 --format ansi"));
    assert!(!log.contains("pane read w1:p2"));
    assert!(log.contains(&format!(
        "plugin pane open --plugin shadowfax.talon --entrypoint picker --placement popup --width 95% --height 90% --env HERDR_TALON_RUN_ID={run_id} --focus"
    )));
}

#[test]
fn empty_recent_capture_falls_back_to_the_visible_source() {
    let (fake_dir, herdr) = fake_herdr();
    std::fs::write(fake_dir.path().join("herdr.empty-recent"), "").unwrap();
    let state = tempdir().unwrap();
    let store = RunStore::new(state.path()).unwrap();
    let context = InvocationContext::parse(r#"{"focused_pane_id":"w1:p1"}"#).unwrap();

    let LaunchOutcome::Opened { run_id } =
        launch(&herdr, &context, &Config::default(), &store).unwrap()
    else {
        panic!("expected popup launch");
    };
    let snapshot = store.claim(&run_id).unwrap();
    let log = std::fs::read_to_string(fake_dir.path().join("herdr.log")).unwrap();

    assert_eq!(snapshot.text, "visible-value\n");
    assert!(snapshot.ansi.contains("\u{1b}[36m"));
    assert!(log.contains("pane read w1:p1 --source visible --format text"));
    assert!(log.contains("pane read w1:p1 --source visible --format ansi"));
}

#[test]
fn no_matches_still_opens_a_manual_selection_popup() {
    let (fake_dir, herdr) = fake_herdr();
    let state = tempdir().unwrap();
    let store = RunStore::new(state.path()).unwrap();
    let context = InvocationContext::parse(r#"{"focused_pane_id":"w1:p1"}"#).unwrap();
    let config = Config {
        alphabet: vec!['a', 's'],
        patterns: vec![PatternDefinition {
            name: "never".into(),
            regex: "ZZZ".into(),
        }],
        ..Config::default()
    };

    let outcome = launch(&herdr, &context, &config, &store).unwrap();
    let log = std::fs::read_to_string(fake_dir.path().join("herdr.log")).unwrap();

    let LaunchOutcome::Opened { run_id } = outcome else {
        panic!("expected popup launch");
    };
    assert!(store.claim(&run_id).unwrap().targets.is_empty());
    assert!(log.contains("plugin pane open"));
    assert!(!log.contains("notification show"));
}

#[test]
fn invalid_config_notifies_and_fails_before_opening() {
    let (fake_dir, herdr) = fake_herdr();
    let state = tempdir().unwrap();
    let config = state.path().join("config.toml");
    std::fs::write(&config, "[[patterns]\n").unwrap();

    let error = launch_with_reporting(
        &herdr,
        r#"{"focused_pane_id":"w1:p1"}"#,
        &config,
        state.path(),
    )
    .unwrap_err()
    .to_string();
    let log = std::fs::read_to_string(fake_dir.path().join("herdr.log")).unwrap();

    assert!(error.contains("failed to parse"));
    assert!(log.contains("notification show Talon --body Talon could not start:"));
    assert!(!log.contains("plugin pane open"));
}
