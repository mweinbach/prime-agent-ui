# Desktop changes

## 0.10.2

- Select and copy transcript text, follow Markdown links, and expand syntax-highlighted code, diffs, and terminal output.
- Read earlier messages while a reply streams, then use Jump to bottom to return to it.
- Use keyboard navigation in pickers and the composer, and undo or redo text edits. All three Google Sans font families are bundled.
- The sidebar, inspector, and dialogs now animate. Enable Reduce motion in Settings for immediate transitions.

- Browse a project without switching chats. The sidebar now previews five chats per folder, lists unsent drafts, and keeps other CLI directories under Folder history.

- Build native archives for Windows ARM64 and x86-64, Apple Silicon macOS, and x86-64 Linux with `scripts/build_desktop.py`.

- The desktop app now builds the companion CLI and bundles its runtime and skills from the pinned [`harness/`](harness/) submodule. Existing checkouts must run `make init` after pulling this change.
- The native macOS app supports project-scoped drafts, CLI conversations and credentials, and live subagent inspection. See the [desktop guide](crates/pa-desktop/README.md).

For CLI and daemon changes, see the [harness changelog](harness/CHANGELOG.md).
