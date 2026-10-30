# flamenote

Floating notes from selected text, in the spirit of Flameshot.

When it starts, the program lives in the tray. A global hotkey arms selection: the cursor turns into a crosshair and the toolbox does not appear yet. At that point, Esc only restores the cursor. After you select text in the focused window, the toolbox appears next to that selection. The toolbox has two fixed tools: the first puts the text on a light, rounded-corner note that always stays above other windows; the second saves the text to a `.txt` file in the destination folder and leaves that file on the clipboard, ready to paste. Configured queries add up to three buttons that send the text, with a preprompt, to an OpenAI-compatible server. The note can be dragged, resized from its edges, and its text can be edited. The corner icons copy all content, toggle editing, and open an eight-color palette. Double-clicking closes the note. With the toolbox already open, Esc closes only the toolbox.

## Requirements

This version targets X11. It does not run on Wayland.

The tray uses the classic systray protocol (XEmbed), the same one used by i3bar with `tray_output`. Without a tray, the hotkey still works and the program reports this once.

GTK 3 draws the windows. With a compositor (picom, for example), the shadow around the card is visible. Without a compositor, flamenote clips the corners with a mask so no black square is left behind.

## Build and run

```bash
cargo build --release
./target/release/flamenote
```

On i3, add this autostart line:

```
exec --no-startup-id flamenote
```

While the first instance is running, a second invocation arms selection instead of creating another process. If the instance lock is already held and the request cannot reach the main process, the program prints a message saying it is already running and exits.

## Configuration

The file lives at `$XDG_CONFIG_HOME/flamenote/config.toml`, or at `~/.config/flamenote/config.toml`. `FLAMENOTE_CONFIG` points to another path. On first run, the program writes the defaults if the file does not exist yet, and reloads it every 2 seconds.

```toml
# flamenote global hotkey.
# Modifiers: Ctrl, Alt, Shift, Super.
# Examples: "Super+Shift+N", "Ctrl+Alt+N", "F9"
hotkey = "Super+Shift+N"
# Folder where selected text is saved as a file.
# Accepts an absolute path or a path starting with ~.
copies_dir = "~/copias"

# Initial note size, in pixels. The edges can also be dragged.
[note]
width = 320
height = 220

[llm]
base_url = "http://localhost:11434"
model = "qwen3:14b"
api_key = ""

[[queries]]
prompt = "Summarize in one sentence:"
destination = "clipboard"

[[queries]]
prompt = "List the tasks:"
destination = "file"
```

The default hotkey is `Super+Shift+N`. Esc cannot be the hotkey. During selection, Esc only restores the cursor. With the toolbox open, Esc closes the toolbox. An invalid hotkey falls back to the default and the message is printed to the terminal.

The default destination folder is `~/copias`. It is created when the first file is saved. Relative paths are resolved from the home folder. Changes to the file are reloaded every 2 seconds.

`[note]` defines the initial note size. `width` and `height` are the card width and height in pixels; accepted values range from 220 to 1600 for width and from 160 to 1200 for height. The initial aspect ratio is the ratio between the two and is maintained when zoom changes.

`[llm]` accepts any server with the `/v1/chat/completions` endpoint in OpenAI format. `base_url` may end with `/v1` or already include the full path; without a suffix, flamenote appends `/v1/chat/completions`. Leave `api_key` empty on local servers such as Ollama. Up to three `[[queries]]` can be defined. The `prompt` is sent as a system message and the selected text as the user message. `destination = "file"` saves the response to the destination folder and leaves the file on the clipboard; `destination = "clipboard"` copies the response text. The output limit is 4096 tokens; a response cut off by the server is not saved and triggers a truncation notification. The configuration file stores the API key in plain text but is written with `600` permissions.

Right-clicking the tray opens Settings. Clicking the hotkey field captures the next key combination. It is written to that file immediately and takes effect right away. The destination folder field saves the folder when you press Enter or leave the field; leaving it empty restores `~/copias`. On the LLM tab, the button that saves the LLM configuration stores the URL, model, key, and the three queries. Queries with an empty prompt are disabled.

`FLAMENOTE_DEBUG=1` writes what the X11 thread is doing to the terminal.

## What this version does

- The hotkey, a left click on the tray, and the “Select text” menu item arm selection. The system cursor disappears and a crosshair follows the pointer. The crosshair does not receive clicks, so dragging in the window underneath still works. The toolbox does not open at that point. Text that was already selected before the hotkey does not open the toolbox.
- With the crosshair on screen, Esc restores the cursor and does not close notes. Pressing the hotkey again also cancels.
- After a drag that selects text, or a new selection in the focused window, the toolbox appears next to the end of the selection. The tools start enabled with that text. They become disabled if the selection disappears before the click. Whitespace counts as nothing selected.
- The tools read the X11 PRIMARY selection. The selection owner must be the focused window, a relative of it in the window tree, another client of the same process (`_NET_WM_PID`), or the same X11 connection. This catches the invisible window where GTK and Chromium store the selection. Two windows from the same process can enable the tools even when the selection is in the window that is not focused.
- The toolbox does not steal focus. Clicking outside it does not close it. It closes on Esc, the hotkey again, a left click on the tray, or when a tool is used.
- Right-clicking the tray shows “Select text”, “Settings”, and “Quit”.
- The note shows the captured text, up to 20,000 characters. It starts at the size defined in `[note]`. You can drag it and resize it by dragging its edges or corners. A movement smaller than about 2 pixels does not count as a drag, so double-clicking still closes the note.
- The note is a floating window managed by the window manager: it stays on the workspace where it was created and disappears when you switch workspaces. To move it to another workspace, use the normal i3 shortcuts for moving windows.
- When the text does not fit the current size, a vertical scrollbar appears. The text wraps to fit the width. The mouse wheel scrolls the text.
- The copy icon in the top-right corner puts all the note text on the clipboard. The pencil icon toggles text editing; Esc also leaves editing. The palette icon shows eight colors; when you choose one, the palette closes and the entire card takes on the chosen color.
- Ctrl with the mouse wheel increases or decreases the note and its text together while keeping the current note aspect ratio. Without Ctrl, the wheel keeps scrolling the text.
- The file tool uses up to 30 characters from the selection as the name, replacing spaces with `_` and discarding other characters. The text is written with a `.txt` extension in the destination folder and the file is copied as a pasteable item (`x-special/gnome-copied-files`), with a notification. A repeated name overwrites the previous file.
- Each configured query gets a button with a chat icon; the tooltip shows the preprompt. When clicked, the toolbox closes, the cursor is released, and the request runs in the background. When it finishes, the response is saved as a file or copied as text according to the query checkbox, and a notification reports the result or the error.

## License

MIT. Copyright (c) 2026 EduMelo.
