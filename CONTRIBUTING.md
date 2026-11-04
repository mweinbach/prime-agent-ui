# Contributing

Develop the GUI on a branch of `mweinbach/prime-agent-ui`. Run `make init`, then `make check`. For a shipped build, run `make desktop-app` on macOS. Tests must not activate a window on the user’s desktop or access their credentials or conversations.

## Harness changes

A recursive clone checks out `harness/` at a detached, tested commit. Do not commit there without creating a branch. Create a branch in the submodule, run its own checks, push it to `mweinbach/prime-agent`, and merge it into that fork’s `main` first. In the desktop repo, follow the [pin update instructions](README.md#how-the-repositories-fit-together). Do not add copies of harness crates, skills, or the Python runtime to the desktop repository.

To bring public upstream changes into the fork, work in a separate checkout of `mweinbach/prime-agent` (or on a named branch in `harness/`). Add `https://github.com/PrimeIntellect-ai/prime-agent.git` as `upstream` if needed, fetch `upstream/main`, and merge it into a branch based on `origin/main`. Review and test that merge before updating the fork. Do not force-reset the fork to upstream: the fork carries provider and desktop compatibility changes.

The root Cargo workspace owns the UI and its lockfile. `harness/` is its own workspace with its own lockfile, license notices, and release process. When changing the pin, test both the resulting desktop integration and the affected harness code.
