# Desktop changes

## 0.11.0

- The desktop now runs on Rune. It keeps Google Sans and the light and dark themes, with simpler project and chat sidebars and a quieter inspector.
- Search models by name or provider and choose reasoning effort from the composer, even before sending the first message. Switching chats preserves the unsent draft when browsing prompt history.
- Preview attached images, text, and code before sending. The composer sends the contents you previewed, even if the source file changes.
- Connect an inference provider from Settings, the model picker, or `/login`. Prime Inference supports browser sign-in, API keys, and team selection. OpenAI Codex, Anthropic, GitHub Copilot, and xAI support subscription sign-in. Desktop shares credentials with the selected CLI profile.

## 0.10.3

- Apple Silicon Mac releases support signed Sparkle updates through the Prime Agent menu. Install this version manually once when upgrading from 0.10.2.

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
