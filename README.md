<div align="center">

# 🦅 Talon

**Browse focused terminal history and copy values with short keyboard hints.**

*A fast, wrapped history picker built natively for Herdr.*

[![Herdr 0.7.5+](https://img.shields.io/badge/Herdr-0.7.5%2B-6c71c4)](https://herdr.dev)
[![Rust](https://img.shields.io/badge/built%20with-Rust-b7410e)](https://www.rust-lang.org/)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)

</div>

Press `prefix+g` in any terminal pane. Talon freezes up to 1,000 recent rendered terminal rows from that pane, unwraps soft wraps, and opens the result in a centered native popup. Detected paths, URLs, hashes, IPs, and other useful values receive compact hints; type one to copy its value and close immediately.

The popup also works as a keyboard-driven history viewer when no value is detected:

- **Focused by design** — only the pane where Talon was invoked is captured, whether it is tiled or maximized.
- **Clean reflow** — long logical lines wrap to the popup width and reflow in place after a resize.
- **Exact visual copy** — characterwise and linewise selections preserve real source newlines without inventing newlines at soft wraps.
- **Viewport-local hints** — only visible targets receive labels, keeping hints short even across a large history capture.
- **Searchable history** — `/`, `n`, and `N` make older output quick to find.
- **Responsive popup** — laptop, normal, and full-ultrawide widths keep the picker readable.
- **Frozen and private** — the source can keep running while a one-shot owner-only snapshot is open.

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
description = "Browse pane history with Talon"
```

Existing config is preserved. Before changing it, Talon writes a sibling `config.toml.talon-backup-*` file. Repeating the installer is a byte-for-byte no-op; if another built-in or custom action owns `prefix+g`, Talon stops without replacing it.

### Upgrading from 0.1

Talon 0.2 replaces the in-pane overlay with a modal history popup. After upgrading, invoke `shadowfax.talon.install-keybindings` again so the managed binding matches the current plugin contract.

`prefix+g` opens the popup; it does not toggle an open popup. Herdr sends every key to the active modal, so copy a value or close Talon with `q`, `Esc`, or `Ctrl-c` before invoking the shortcut again.

## Workflow

Talon opens at the newest captured output. The footer always shows the active mode and its important controls.

### Browse

| Key | Result |
| --- | --- |
| visible hint | Copy that target and close |
| arrows or `h`, `j`, `k`, `l` | Move through wrapped history |
| `PageUp`, `PageDown` | Move one page |
| `Ctrl-u`, `Ctrl-d` | Move half a page |
| `g`, `G` | Jump to the beginning or end |
| `0`, `$` | Move to the beginning or end of a logical line |
| `v`, `V` | Begin character or line selection |
| `/` | Search the capture |
| `n`, `N` | Go to the next or previous search match |
| `Esc`, `q`, or `Ctrl-c` | Close |

Hints are reassigned deterministically when the viewport changes. Partial hint input is cleared by navigation, so the labels on screen are always the complete active set.

### Select and copy

After `v` or `V`, use the same movement keys to extend the selection.

| Key | Result |
| --- | --- |
| `y` | Copy the exact selection and close |
| `Esc` | Cancel the selection and return to Browse |

A selection can cross any number of wrapped visual rows. Talon copies a newline only where the original captured output had one.

### Search

Press `/`, type a case-sensitive query, then press `Enter`. The first match at or below the current logical line is previewed as the query changes. After accepting, use `n` and `N` to cycle with wraparound. `Esc` cancels the search and restores the original cursor.

If `pbcopy` fails, Talon keeps the popup open and shows the error in the footer so the selection is not lost.

## Recognized targets

Talon includes patterns for:

- URLs, IP addresses, UUIDs, hexadecimal values, long numbers, and SHA-like hashes
- absolute, relative, and home-relative paths, including `path:line` values
- Git SSH remotes, status paths, branch names, and diff paths
- common Kubernetes resource names

Repeated values share one target. Overlapping matches resolve deterministically: the leftmost match wins, then earlier pattern priority. Custom patterns are ordered before built-ins.

## Configure

Create a config from [talon.toml.example](talon.toml.example):

```sh
talon_config_dir="$(herdr plugin config-dir shadowfax.talon)"
mkdir -p "$talon_config_dir"
cp talon.toml.example "$talon_config_dir/config.toml"
```

Configuration is loaded on every launch, so edits do not require a Herdr reload.

### Hints and patterns

The default hint alphabet is `asdfwerzxcuioptbm`. An alphabet must contain at least two unique lowercase ASCII letters and cannot contain the normal-mode keys `g`, `h`, `j`, `k`, `l`, `n`, `q`, `v`, or `y`. Talon migrates the exact 0.1 default automatically; custom 0.1 alphabets must remove newly reserved keys before launch.

`enabled_builtin_patterns` accepts:

```text
ip, uuid, sha, digit, url, path, hex,
kubernetes, git-status, git-status-branch, diff
```

Add ordered regular expressions with `[[patterns]]`:

```toml
[[patterns]]
name = "hostname"
regex = '(?i)\b(?:[a-z0-9](?:[a-z0-9-]{0,61}[a-z0-9])?\.)+[a-z]{2,63}\b'

[[patterns]]
name = "ticket"
regex = "TKT-[0-9]+"

[[patterns]]
name = "captured-ticket"
regex = "ticket=(?<match>TKT-[0-9]+)"
```

When a regex has a named `match` capture, Talon labels and copies only that capture.

### Popup size

The default popup is 90% of the client width and height. Profiles are checked in order; the first matching profile wins.

```toml
[popup]
width = "90%"
height = "90%"

[[profiles]]
name = "laptop"
max_client_width = 310
width = "95%"
height = "90%"

[[profiles]]
name = "partial-ultrawide"
max_client_width = 350
width = "90%"
height = "90%"

[[profiles]]
name = "full-ultrawide"
min_client_width = 400
width = "70%"
height = "90%"
```

Widths from 351 through 399 cells use the default. Popup dimensions can be percentages from 1% through 100% or quoted positive cell counts such as `width = "120"`.

## How it works

Talon reads plain and ANSI `recent-unwrapped` output from the invoking pane. Plain text supplies stable match and selection coordinates; ANSI spans preserve terminal styling when the two representations align. A custom terminal-cell wrap table maps every visual row back to its logical source range, so resize, highlight, search, and selection all use one coordinate model.

The launch action records the frozen capture in a private one-shot handoff and opens the picker as a focused Herdr popup. The picker claims and removes the handoff, writes successful completions through `pbcopy`, then exits so Herdr dismisses the popup. Herdr routes every key to an open modal popup, so close Talon with `q`, `Esc`, or `Ctrl-c`; a launch attempted while another modal is open fails safely without closing it.

## Limits

- Herdr caps the capture at 1,000 recent rendered terminal rows before unwrapping soft wraps.
- The snapshot is frozen; output produced after launch is intentionally absent.
- Talon does not change the source pane's scroll or zoom state and does not reuse Herdr's native Copy mode.
- Completion copies to the clipboard only. It does not paste, open, submit, or execute the selected text.
- Selection is keyboard-driven; mouse selection is not implemented.

## Local development

```sh
git clone https://github.com/shadowfax92/herdr-talon.git
cd herdr-talon
cargo build --release --locked
herdr plugin link . --enabled
herdr plugin action invoke shadowfax.talon.install-keybindings
```

Run the complete local gate:

```sh
cargo fmt --all -- --check
cargo clippy --locked --all-targets --all-features -- -D warnings
cargo test --locked --all-targets --all-features
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
