# Prime Agent Desktop

An unofficial native macOS client for [Prime Agent](https://github.com/PrimeIntellect-ai/prime-agent), built with [GPUI](https://www.gpui.rs/). It uses the Prime Agent daemon and reads the same saved chats and provider credentials as the CLI. Credentials and conversations are not distributed in this repository.

## Build on macOS

Install Rust, Xcode, and the Metal toolchain. Clone the desktop app **with its submodule**:

```sh
git clone --recurse-submodules https://github.com/mweinbach/prime-agent-ui.git
cd prime-agent-ui
make desktop-app
open "dist/Prime Agent.app"
```

For an existing checkout, run `make init` before building. `make build` builds the UI and companion CLI in `target/release`. `make test` runs the desktop tests headlessly. `make run-desktop` launches the GUI from source. See the [desktop guide](crates/pa-desktop/README.md) for projects, subagents, CLI data, and keyboard shortcuts. The local build is ad-hoc signed, not notarized.

## How the repositories fit together

| Repository | Owns |
| --- | --- |
| [mweinbach/prime-agent-ui](https://github.com/mweinbach/prime-agent-ui) | The GPUI app in `crates/pa-desktop`, integration tests, and macOS packaging |
| [mweinbach/prime-agent](https://github.com/mweinbach/prime-agent) | The `harness/` submodule: Rust daemon, CLI, providers, Python runtime, and skills |
| [PrimeIntellect-ai/prime-agent](https://github.com/PrimeIntellect-ai/prime-agent) | Upstream for the harness fork |

Each desktop commit pins an exact harness commit. Both the Cargo path dependencies and the packaged CLI, runtime, and skills use that checkout. The tracked update branch is the fork’s `main`, not the upstream organization’s `main`. Building does **not** pull new code automatically.

**Pull the desktop app and its tested harness:**

```sh
git pull --ff-only --recurse-submodules
make init
```

**Update the pin to a newer fork commit:** Start with a clean submodule, then run:

```sh
git submodule update --init --remote harness
git -C harness log -1 --oneline
git diff --submodule=log
make check
make desktop-app
git add harness
git commit -m "Update the pinned Prime Agent harness"
```

If a dependency change makes the desktop lockfile stale, run `cargo tree -p pa-desktop` to update it, review `Cargo.lock`, and include it with the pin. Push changes to the harness fork **before** publishing a desktop pin that references them. See [Contributing](CONTRIBUTING.md) for making harness changes and bringing upstream fixes into the fork.

**Licenses:** [Prime Agent](LICENSE) · [Desktop icons](crates/pa-desktop/ICON-LICENSE) · [Harness and export assets](harness/crates/pa-core/assets/export-html/NOTICE.md).
