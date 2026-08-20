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
  elif [ "$source" = "recent-unwrapped" ] && [ -e "$0.whitespace-recent" ]; then
    printf '   \n'
  elif [ "$source" = "visible" ]; then
    if [ "$format" = "ansi" ]; then printf '\033[36mvisible-value\033[0m\n'; else printf 'visible-value\n'; fi
  else
    if [ "$format" = "ansi" ]; then printf '\033[33mdeadbeef\033[0m\n'; else printf 'deadbeef\n'; fi
  fi
elif [ "$1 $2 $3" = "plugin pane open" ]; then
  if [ -e "$0.ui-busy" ]; then
    printf '%s\n' '{"id":"cli:plugin","error":{"code":"ui_busy","message":"another modal is active"}}' >&2
    exit 1
  else
    printf '%s\n' '{"id":"cli:plugin","result":{"type":"ok"}}'
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
fn concurrent_launch_preserves_the_existing_popup_and_removes_its_failed_handoff() {
    let (fake_dir, herdr) = fake_herdr();
    let state = tempdir().unwrap();
    let store = RunStore::new(state.path()).unwrap();
    let context = InvocationContext::parse(r#"{"focused_pane_id":"w1:p1"}"#).unwrap();

    let LaunchOutcome::Opened { run_id } =
        launch(&herdr, &context, &Config::default(), &store).unwrap();
    store.claim(&run_id).unwrap();
    std::fs::write(fake_dir.path().join("herdr.ui-busy"), "").unwrap();

    let error = launch(&herdr, &context, &Config::default(), &store)
        .unwrap_err()
        .to_string();

    let log = std::fs::read_to_string(fake_dir.path().join("herdr.log")).unwrap();
    assert!(error.contains("ui_busy"));
    assert_eq!(
        log.lines()
            .filter(|line| line.starts_with("plugin pane open "))
            .count(),
        2
    );
    assert!(!log.contains("plugin pane close"));
    assert_eq!(std::fs::read_dir(store.root()).unwrap().count(), 0);
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

    let LaunchOutcome::Opened { run_id } =
        launch(&herdr, &context, &Config::default(), &store).unwrap();
    let snapshot = store.claim(&run_id).unwrap();
    let log = std::fs::read_to_string(fake_dir.path().join("herdr.log")).unwrap();

    assert_eq!(snapshot.source_pane_id, "w1:p1");
    assert_eq!(snapshot.text, "deadbeef\n");
    assert!(snapshot.ansi.contains("\u{1b}[33m"));
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
        launch(&herdr, &context, &Config::default(), &store).unwrap();
    let snapshot = store.claim(&run_id).unwrap();
    let log = std::fs::read_to_string(fake_dir.path().join("herdr.log")).unwrap();

    assert_eq!(snapshot.text, "visible-value\n");
    assert!(snapshot.ansi.contains("\u{1b}[36m"));
    assert!(log.contains("pane read w1:p1 --source visible --format text"));
    assert!(log.contains("pane read w1:p1 --source visible --format ansi"));
}

#[test]
fn whitespace_only_recent_capture_is_preserved() {
    let (fake_dir, herdr) = fake_herdr();
    std::fs::write(fake_dir.path().join("herdr.whitespace-recent"), "").unwrap();
    let state = tempdir().unwrap();
    let store = RunStore::new(state.path()).unwrap();
    let context = InvocationContext::parse(r#"{"focused_pane_id":"w1:p1"}"#).unwrap();

    let LaunchOutcome::Opened { run_id } =
        launch(&herdr, &context, &Config::default(), &store).unwrap();
    let snapshot = store.claim(&run_id).unwrap();
    let log = std::fs::read_to_string(fake_dir.path().join("herdr.log")).unwrap();

    assert_eq!(snapshot.text, "   \n");
    assert!(!log.contains("pane read w1:p1 --source visible"));
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

    let LaunchOutcome::Opened { run_id } = launch(&herdr, &context, &config, &store).unwrap();
    let log = std::fs::read_to_string(fake_dir.path().join("herdr.log")).unwrap();

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
