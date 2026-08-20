use std::path::PathBuf;
use std::process::Command;

#[test]
fn cli_requires_a_subcommand() {
    let status = Command::new(env!("CARGO_BIN_EXE_herdr-talon"))
        .status()
        .unwrap();

    assert!(!status.success());
}

#[test]
fn cli_rejects_an_unknown_subcommand() {
    let status = Command::new(env!("CARGO_BIN_EXE_herdr-talon"))
        .arg("unknown")
        .status()
        .unwrap();

    assert!(!status.success());
}

#[test]
fn manifest_declares_the_complete_plugin_contract() {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("herdr-plugin.toml");
    let manifest: toml::Value = toml::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();

    assert_eq!(manifest["id"].as_str(), Some("shadowfax.talon"));
    assert_eq!(manifest["version"].as_str(), Some("0.2.0"));
    assert_eq!(
        manifest["description"].as_str(),
        Some("Browse focused pane history and copy values with keyboard hints.")
    );
    assert_eq!(manifest["min_herdr_version"].as_str(), Some("0.7.5"));
    assert_eq!(
        manifest["platforms"].as_array().unwrap()[0].as_str(),
        Some("macos")
    );
    assert_eq!(manifest["build"].as_array().unwrap().len(), 1);
    assert_eq!(manifest["actions"].as_array().unwrap().len(), 2);
    assert_eq!(manifest["panes"].as_array().unwrap().len(), 1);
    assert_eq!(manifest["panes"][0]["placement"].as_str(), Some("popup"));
    assert_eq!(manifest["panes"][0]["width"].as_str(), Some("90%"));
    assert_eq!(manifest["panes"][0]["height"].as_str(), Some("90%"));
}
