<div align="center">

# 🦅 Talon

**Select visible terminal values in Herdr with short keyboard hints.**

*A fast, spatial target picker inspired by tmux-fingers and built natively for Herdr.*

[![Herdr 0.7.5+](https://img.shields.io/badge/Herdr-0.7.5%2B-6c71c4)](https://herdr.dev)
[![Rust](https://img.shields.io/badge/built%20with-Rust-b7410e)](https://www.rust-lang.org/)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)

</div>

Press `prefix+g` and Talon freezes the current Herdr tab into a temporary overlay. Recognized targets across every visible pane receive compact hints; type a hint to copy, paste, open, or collect its value without reaching for the mouse.

- **Spatial hints** — labels appear directly beside paths, URLs, hashes, IPs, and other useful values.
- **Progressive matching** — hints narrow as you type and stay short for common target counts.
- **Four completion modes** — copy, paste without Enter, open with macOS, or collect several values.
- **Deterministic overlap handling** — custom patterns win, repeated values share one hint, and results stay stable.
- **Frozen geometry** — the overlay matches exactly what you saw when Talon launched.
- **Safe key installation** — the installer preserves unrelated config and refuses key conflicts.

## Install

Requires macOS, [Herdr](https://herdr.dev) 0.7.5 or newer, and a Rust toolchain.

```sh
herdr plugin install shadowfax92/herdr-talon
herdr plugin action invoke shadowfax.talon.install-keybindings
```

The installer adds this command to `~/.config/herdr/config.toml` and reloads Herdr:

```toml
[[keys.command]]
key = "prefix+g"
type = "plugin_action"
command = "shadowfax.talon.launch"
description = "Select visible target with Talon"
```

Existing config is preserved. Before changing it, Talon writes a sibling `config.toml.talon-backup-*` file. Repeating the installer is a byte-for-byte no-op; if another built-in or custom action already owns `prefix+g`, Talon stops without replacing it.

## Keys

| Key | Result |
| --- | --- |
| hint | Copy the target |
| `Shift` + final hint key | Copy, then paste into the originating pane without Enter |
| `Ctrl` + final hint key | Copy, then open with macOS `open` |
| `Tab`, hints, `Tab` | Copy multiple targets, joined by spaces |
| `prefix+g` | Close the active picker |
| `Esc`, `q`, or `Ctrl-c` | Cancel |

Multi-select preserves selection order and ignores duplicates. Resizing the terminal cancels the picker because the frozen tab geometry is no longer valid. Internal pane resizes while Herdr opens the overlay are ignored.

## Recognized targets

Talon includes patterns for:

- URLs, IP addresses, UUIDs, hexadecimal values, long numbers, and SHA-like hashes
- absolute, relative, and home-relative paths, including `path:line` values
- Git SSH remotes, status paths, branch names, and diff paths
- common Kubernetes resource names

Repeated values share one hint. Overlapping matches resolve deterministically: the leftmost match wins, then earlier pattern priority. Custom patterns are ordered before built-ins.

## Configure

Create a config from [talon.toml.example](talon.toml.example):

```sh
talon_config_dir="$(herdr plugin config-dir shadowfax.talon)"
mkdir -p "$talon_config_dir"
cp talon.toml.example "$talon_config_dir/config.toml"
```

`alphabet` must contain at least two unique lowercase ASCII letters and cannot contain `q`. `enabled_builtin_patterns` accepts:

```text
ip, uuid, sha, digit, url, path, hex,
kubernetes, git-status, git-status-branch, diff
```

Add ordered regular expressions with `[[patterns]]`:

```toml
[[patterns]]
name = "ticket"
regex = "TKT-[0-9]+"

[[patterns]]
name = "captured-ticket"
regex = "ticket=(?<match>TKT-[0-9]+)"
```

When a regex has a named `match` capture, Talon labels and copies only that capture. Configuration is loaded on every launch, so plugin-config edits do not require a Herdr reload.

## How it works

Talon asks Herdr for the visible tab layout and rendered pane contents, records a private one-shot handoff, and opens its picker as a native Herdr overlay. The picker paints the frozen ANSI backdrop, adds hint labels across the tab, and deletes the handoff after claiming it.

Completions are deliberately narrow:

- copy uses `pbcopy`;
- paste copies first, then asks Herdr to insert text without Enter;
- open copies first, resolves an existing relative path against the source pane cwd, then invokes macOS `open`.

## Limits

Talon works from Herdr's visible-pane API. It does not inspect scrollback, preserve Copy mode, jump the source pane, or execute arbitrary shell actions. Output arriving after launch is intentionally absent from the frozen overlay.

## Local development

```sh
git clone https://github.com/shadowfax92/herdr-talon.git
cd herdr-talon
herdr plugin link .
```

Run the complete local gate:

```sh
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test --locked
cargo build --release --locked
```

After changing the managed binding contract, invoke `shadowfax.talon.install-keybindings` again.

## Remove

Delete Talon's `[[keys.command]]` block from `~/.config/herdr/config.toml`, then run:

```sh
herdr server reload-config
herdr plugin uninstall shadowfax.talon
```

## Attribution

Talon is behaviorally inspired by [tmux-fingers](https://github.com/Morantron/tmux-fingers). The implementation and built-in target categories are independent; see [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md).

## License

[MIT](LICENSE)
