# pa-desktop

`prime-agent-ui` is the native Rune desktop app for Prime Agent. It talks to the `prime-agent` daemon built from the pinned [`harness/`](../../harness) submodule.

For Windows and Linux builds, see [Download or build](../../README.md#download-or-build). On those platforms the app uses Ctrl in place of the Command key shown in the macOS shortcuts below.

## Build and run on macOS

Run these commands from the repository root:

```bash
make init
make desktop-app
open "dist/Prime Agent.app"
```

To run from the checkout without packaging the app, use `make run-desktop`. Both binaries share `target/`. The development launcher points the daemon at `harness/` for its Python runtime and built-in skills. The app bundle carries its own copies and does not need the checkout.

The desktop embeds Google Sans for prose, Google Sans Code for code and telemetry, and Google Sans Flex for headings. All three families use the [SIL Open Font License](FONT-LICENSE).

## Projects and chats

Use the folder-plus button (`⇧⌘O`) to add a project. It opens that folder’s composer without creating a CLI session. Your added folders and the current working folder appear first. Other directories from the CLI sit under **Folder history**. Hover over a folder and click the pin to keep it with your projects without leaving your conversation. Unpinning never deletes files or chats.

Click the chevron to preview up to five chats, or click the folder name to browse all its loaded chats. Browsing a project filters the sidebar without switching the conversation or moving its draft. **All projects** takes you back. Click the folder in the chat header to reveal the open conversation’s project. Selecting a project in the Quick Switcher also opens this view. Search includes chats outside the preview and opens matching history folders; within a project it searches just that folder’s group. **Load older chats** fetches the next page from the CLI store.

Use the pen beside a folder to compose there. The toolbar pen and `⌘N` compose in the project you’re browsing, or in the current conversation’s project when no project is in focus. Unsent drafts stay in memory and appear under their original folders. Sending a message or requesting a model for a new chat creates a session.

Live and saved conversations appear together. A chat under an added folder belongs to its nearest added parent, but always reopens in its original working directory. Saved chats from temporary directories without an active chat, draft, or added project appear under **Temporary chats** in Folder history. Hover over a chat to see its working directory.

## Preview attachments

Use **+** in the composer to choose a file, or drop several files onto the composer. PNG, JPEG, GIF, and WebP images show a thumbnail. UTF-8 text, including source code and Markdown, shows a snippet. Select a card for a larger preview. Its separate remove button takes it out of the draft.

Previews use the contents captured when you attached the files. Sending uses the original image bytes and the full text, even when the preview shows only an excerpt. Each draft can hold up to 16 files totaling 15 MiB. File text and your message together must fit the 1 MiB prompt limit. Images are limited to 16,384 pixels per side and 33,554,432 pixels in total for decoding. Animated images preview their first frame.

## Subagents

When a chat spawns a subagent, **Subagents** in the chat header opens **Inspector → Agents**. The list belongs to the current chat, not the whole project. Cards show the reported run status, task and result. Progress notes and tool counts appear when the worker reports them. **Inspect live** opens a resident child’s chat; **Parent** returns to its parent. For nested agents, inspect the child and open its own Agents tab.

You can message a resident child, cancel a running child, or remove a settled child. A retained result can still be shown when the worker is no longer live. Subagents do not become separate projects in the sidebar.

## Use your Prime Agent CLI data

Desktop opens the same data as `prime-agent`: `~/.prime/agent` by default, or `PRIME_AGENT_CODING_AGENT_DIR` when set in its environment. There is no second copy of your chats or credentials.

- **Conversations:** The project sidebar lists conversations directly from the CLI store. Opening one resumes its original file.
- **Credentials and models:** Open **Settings** (`⌘,`) to see stored API keys and OAuth sign-ins by provider, including custom providers from `models.json`. Key values and tokens aren't displayed. Editing a saved key changes it for both clients. Shell-only environment variables may not reach an app opened from Finder. Launch the binary from that shell to inherit them, or save a key through the CLI.
- **Custom data folder:** In **Settings → Prime Agent CLI data → Choose folder…**, select the agent directory that contains `auth.json`, `sessions/` or `settings.json`, not a project checkout. Desktop remembers the folder on this Mac and uses a dedicated daemon for it. **Use default** restores the original location. `PRIME_AGENT_SESSION_DIR` still overrides the sessions location when supplied at launch.
- **Live CLI sessions:** In the default data folder, choose **Attach to CLI daemon** in Settings to join an existing terminal session. Start `prime-agent` in a terminal first. Desktop won't start, stop or replace the CLI daemon. If the CLI already holds a saved chat, join its daemon or close the CLI session before resuming it in Desktop.

Global agent settings, model configuration and the continual harness stay in the CLI data folder. Project settings still follow the conversation’s project. Desktop-only preferences are saved separately in `desktop-ui.json`.

Project list telemetry is off unless you explicitly set `telemetry.enabled` to `true` in the shared global settings. Desktop respects project opt-outs, `DO_NOT_TRACK` and offline mode; environment overrides cannot enable these events. It records only add/remove actions and standard telemetry metadata, never the project name, path, or chats.

## Test without opening a window

```bash
make test
```

The composer test uses Rune’s headless `Runtime`. It selects a session, clicks the composer, types a prompt and presses Return. A real daemon in a temporary directory runs the scripted `faux` provider. The test checks the draft, prompt and reply without opening the app, sending OS input, or using provider credentials.

The kernel-backed subagent test requires an already provisioned Prime Agent kernel interpreter. To exercise live spawn, completion, and parent/child navigation without opening a window:

```bash
PA_E2E_KERNEL_PYTHON="$HOME/.prime/agent/kernel-venv/bin/python" nice -n 10 cargo test --locked --release -p pa-desktop --test daemon_roundtrip a_spawned_subagent_updates -- --ignored
```

## CLI Flags

- `--cwd <DIR>`: Start in a specific project directory.
- `--daemon-socket <PATH>`: Attach to a daemon already listening at this Unix socket (no automatic launch).
- `--shared-daemon`: Attach to the shared CLI daemon (`daemon.sock`) instead of the dedicated `desktop-daemon.sock`.
- `--dark` / `--light`: Start in *Obsidian Clay* (dark) or *Editorial Parchment* (light).
- `--tab <agents|harness|runtime|system>`: Open a specific Harness Studio tab on launch.
- `--modal <models|switcher|settings>`: Open a specific modal on launch.

## Keyboard Shortcuts

| Shortcut | Action |
| --- | --- |
| `⌘N` | New chat in the browsed project, or the current conversation’s project |
| `⇧⌘O` | Add a project folder |
| `⌘K` | Quick Switcher & command palette |
| `⌘M` | Find a model by name, ID, or provider |
| `⇧⌘E` | Choose a reasoning level supported by the active model |
| `⌘O` | Cycle transcript detail (`Overview` -> `Details` -> `All`) |
| `⌘B` | Toggle the project sidebar |
| `⌘I` | Toggle right Agentic Harness Studio drawer |
| `⇧⌘H` | Open Continual Harness Studio (`Memory`, `Prompt`, `Skill`, `Subagent`) |
| `⌘/` | Toggle `/btw` parallel side-question bar |
| `⇧⌘K` | Compact session context |
| `⇧⌘R` | Trigger Continual Harness refinement |
| `⇧⌘L` | Toggle *Obsidian Clay* / *Editorial Parchment* theme |
| `⌘,` | Provider credentials & daemon target settings |
| `Esc` | Close modal, or stop active streaming turn |

## Scope

- Native GPU-rendered desktop UI built on a pinned revision of [Rune](https://github.com/anomalyco/rune).
- Non-blocking async JSONL daemon bridge (`bridge/`) over `pa-types` wire frames on a 2-thread Tokio worker.
- Dual daemon target support: dedicated `DesktopHarness` supervisor (`desktop-daemon.sock`) and `SharedCli` (`daemon.sock`).
- Continual Harness Studio (`state/harness_store.rs`, `ui/inspector.rs`) backed by `pa-core::refinement` (`<agent_dir>/harness` and `<agent_dir>/session-artifacts/<id>/harness`).
- Provider credential overview and locked `0o600` API-key storage via `pa-core::auth`.

## Non-goals

No duplicate agent loop or duplicate session store; session execution, RLM subagent recursion, Python REPL kernel lifecycle, and compaction run in the supervised `prime-agent` daemon.

## Public API

The `prime-agent-ui` binary entrypoint and `run_desktop_app` launch configuration.

## Depends on

`pa-types`, `pa-core`, `pa-daemon` (socket path resolution), `pa-telemetry` (project adoption event schema).
