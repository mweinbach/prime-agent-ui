# Prime Agent Desktop

An unofficial desktop client for [Prime Agent](https://github.com/PrimeIntellect-ai/prime-agent), built with [Rune](https://github.com/anomalyco/rune). It uses the Prime Agent daemon and reads the same saved chats and provider credentials as the CLI. Credentials and conversations are not distributed in this repository.

To connect an inference provider from the app, open **Settings → Connect provider…** or type `/login` in the composer. See the [desktop guide](crates/pa-desktop/README.md#use-your-prime-agent-cli-data) for sign-in methods and shared CLI credentials.

## Download or build

[Download the latest Apple Silicon Mac app](https://github.com/mweinbach/prime-agent-ui/releases/latest/download/prime-agent-ui-macos-arm64.zip). Unzip it, quit any running copy, and move **Prime Agent.app** into **Applications**.

Beginning with the first update-enabled release (0.10.3), use **Prime Agent → Check for Updates…** or allow Sparkle to check periodically. Users coming from 0.10.2 need to install the new app manually once. The stable [Sparkle appcast](https://github.com/mweinbach/prime-agent-ui/releases/latest/download/appcast.xml) becomes available when the first signed feed is published.

[Tagged releases](https://github.com/mweinbach/prime-agent-ui/releases) are published only when all four platform builds pass. Each archive includes the desktop app, the CLI daemon it was built with, the Python runtime source, and the built-in skills. Extract the whole archive before launching.

| Platform | Archive | Launch |
| --- | --- | --- |
| Windows on Intel/AMD (x86-64) | `prime-agent-ui-windows-x86_64.zip` | `prime-agent-ui.exe` |
| Windows on ARM64 | `prime-agent-ui-windows-arm64.zip` | `prime-agent-ui.exe` |
| macOS on Apple Silicon | `prime-agent-ui-macos-arm64.zip` | `Prime Agent.app` |
| Linux on x86-64 | `prime-agent-ui-linux-x86_64.tar.gz` | `./prime-agent-ui` |

Windows executables are unsigned. The macOS app is ad-hoc signed, not notarized. Linux builds target Ubuntu 24.04; the GUI requires a graphical session (Wayland or X11) and a working graphics driver. These are 64-bit builds, not 32-bit Windows or Intel macOS builds.

### Build from source

Use Git, Python 3.10 or newer, and a native Rust toolchain on the target OS. Install the platform build tools listed below, then run:

```sh
git clone --recurse-submodules https://github.com/mweinbach/prime-agent-ui.git
cd prime-agent-ui
python3 scripts/build_desktop.py
```

On Windows, use `py -3 scripts/build_desktop.py` instead. For an existing checkout, run `git submodule update --init --recursive` first. The script builds both binaries from the checked-in lockfiles, packages them, and writes the archive to `dist/`. `--arch arm64` or `--arch x86_64` checks the host architecture; it does not cross-compile. See the [build workflow](.github/workflows/desktop-build.yml) for the runner setup.

- **Windows:** Install Visual Studio Build Tools with the C++ workload, the MSVC tools for your architecture, and the Windows SDK. Use an MSVC Rust toolchain.
- **macOS:** Install Xcode. `make desktop-app` is still available if you want an unpacked `.app` in `dist/`.
- **Linux:** On Ubuntu 24.04 install the [native build dependencies listed in CI](.github/workflows/desktop-build.yml). Other distributions may use different names for the same libraries.

Maintainers can run the [Desktop build workflow](https://github.com/mweinbach/prime-agent-ui/actions/workflows/desktop-build.yml) to build all four archives without creating a release. Pushing a `release/desktop-v*` candidate branch also builds and verifies the four archives. A `desktop-v*` tag builds the same matrix and publishes the archives to GitHub Releases if every build passes.

See the [desktop guide](crates/pa-desktop/README.md) for projects, subagents, CLI data, and headless tests.

## How the repositories fit together

| Repository | Owns |
| --- | --- |
| [mweinbach/prime-agent-ui](https://github.com/mweinbach/prime-agent-ui) | The Rune app in `crates/pa-desktop`, integration tests, and desktop packaging |
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

**Licenses:** [Prime Agent](LICENSE) · [Desktop icons](crates/pa-desktop/ICON-LICENSE) · [Desktop fonts](crates/pa-desktop/FONT-LICENSE) · [Harness and export assets](harness/crates/pa-core/assets/export-html/NOTICE.md).

### Maintaining the macOS update feed

Before the first signed release, run this on a maintainer Mac with the GitHub CLI signed in and repository Actions secret/variable write access. Start from an up-to-date checkout of `main`:

```sh
./scripts/setup_sparkle.sh --publish
```

This creates or reuses the app-specific Ed25519 identity in your login Keychain, sets the `SPARKLE_PUBLIC_ED_KEY` Actions variable and `SPARKLE_PRIVATE_ED_KEY` Actions secret, and pushes the stable tag for the checked-out desktop version. Omit `--publish` to enroll without tagging. To reuse an existing Sparkle Keychain identity, set `SPARKLE_KEY_ACCOUNT` to its account name. Back up the key securely. Installed copies trust this identity across releases.

Subsequent stable tags publish the signed `appcast.xml` alongside the four tested archives. The release job verifies the signature against the public key embedded in the archived app. Preview releases stay off the stable feed. Developer builds without a signing configuration skip update checks. To build an update-enabled app locally, run `scripts/fetch_sparkle.sh "$PWD/target/sparkle"`, export that path as `SPARKLE_SDK` and export the enrolled public key as `SPARKLE_PUBLIC_ED_KEY` before running `make desktop-app`.
