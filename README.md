# Talon

Talon is a macOS Herdr plugin for selecting visible terminal values with keyboard hints. It is a Rust implementation inspired by tmux-fingers.

Press `prefix+g`. Talon freezes the current tab into a temporary overlay and labels targets in the focused pane. Type a hint to act on its target, or press `prefix+g` again to close the picker.

## Keys

| Key | Result |
| --- | --- |
| hint | Copy the target |
| `Shift` + final hint key | Copy, then paste into the originating pane without Enter |
| `Ctrl` + final hint key | Copy, then open with macOS `open` |
| `Tab`, hints, `Tab` | Copy multiple targets, joined by spaces |
| `prefix+g` | Close the open picker |
| `Esc`, `q`, or `Ctrl-c` | Cancel |

Multi-select keeps selection order and ignores duplicate selections. A terminal resize cancels the picker because its frozen geometry is no longer valid.

## Install

Herdr 0.7.5 or newer and the Rust toolchain are required.

```sh
cd /Users/shadowfax/code/side-projects/herdr-custom-plugins/my/herdr-talon
herdr plugin link .
herdr plugin action invoke shadowfax.talon.install-keybindings
```

The installer adds this binding to `~/.config/herdr/config.toml` and reloads Herdr:

```toml
[[keys.command]]
key = "prefix+g"
type = "plugin_action"
command = "shadowfax.talon.launch"
description = "Select visible target with Talon"
```

Existing config is preserved. Before a change, Talon writes a sibling `config.toml.talon-backup-*` file. Repeating the installer is a byte-for-byte no-op. If another custom command owns `prefix+g`, Talon stops instead of replacing it.

## Targets

Talon recognizes:

- URLs, IP addresses, UUIDs, hexadecimal values, long numbers, and SHA-like hashes
- paths and `path:line` values
- Git SSH remotes, status paths, branch names, and diff paths
- common Kubernetes resource names

Repeated values share one hint. Overlapping patterns resolve deterministically, with custom patterns taking priority.

## Configure

Create the plugin config from the example:

```sh
talon_config_dir="$(herdr plugin config-dir shadowfax.talon)"
mkdir -p "$talon_config_dir"
cp talon.toml.example "$talon_config_dir/config.toml"
```

`alphabet` must contain at least two unique lowercase ASCII letters and cannot contain `q`. `enabled_builtin_patterns` selects from `ip`, `uuid`, `sha`, `digit`, `url`, `path`, `hex`, `kubernetes`, `git-status`, `git-status-branch`, and `diff`.

Add ordered regexes with `[[patterns]]`:

```toml
[[patterns]]
name = "ticket"
regex = "TKT-[0-9]+"

[[patterns]]
name = "captured-ticket"
regex = "ticket=(?<match>TKT-[0-9]+)"
```

When a regex has a named `match` capture, Talon copies and labels only that capture. Config is loaded on every launch, so no reload is needed after plugin-config changes.

## Limits

Talon works from Herdr's visible-pane API. It does not inspect scrollback, preserve Copy mode, jump the source pane, or run arbitrary shell actions. The overlay is a frozen snapshot: output arriving after launch is intentionally absent. Neighboring panes are shown as context, but only the pane that launched Talon receives hints.

Copy uses `pbcopy`; open uses macOS `open`. The plugin manifest therefore supports macOS only.

## Develop and update

Run the complete local gate:

```sh
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
cargo build --release --locked
```

A linked plugin uses this checkout. After pulling or editing code, rebuild it:

```sh
cargo build --release --locked
```

Run `herdr plugin action invoke shadowfax.talon.install-keybindings` again after changing the managed binding contract.

## Remove

Delete Talon's `[[keys.command]]` block from `~/.config/herdr/config.toml`, then run:

```sh
herdr server reload-config
herdr plugin unlink shadowfax.talon
```

Talon handoffs are transient files in its Herdr state directory. The picker claims and removes each handoff once; launch failures and stale-run cleanup also remove them.
