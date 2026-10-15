# MDV

A small native Markdown viewer and editor for **Windows and Linux x86_64**.
Written in Rust with egui, DirectX 12 on Windows, and OpenGL on Linux.
No Electron, webview, or JavaScript runtime.

## Download

Download the archive for your platform from
[Releases](https://github.com/Sockolet/mdv/releases/latest), extract it, and run
`mdv.exe` on Windows or `mdv` on Linux. No Rust installation is needed.

The Linux build targets Ubuntu 22.04 and newer compatible distributions
(glibc 2.35+). It needs an X11 or Wayland desktop, OpenGL 3.3-capable graphics
(software rendering is supported), and the normal desktop libraries. For
Debian/Ubuntu:

```sh
sudo apt install libgl1 libegl1 libxkbcommon0 libwayland-client0 libdbus-1-3 \
  libx11-6 libxcursor1 libxi6 libxrandr2 xdg-utils xdg-desktop-portal zenity
chmod +x mdv
./mdv
```

A desktop-specific portal backend (for example `xdg-desktop-portal-gtk` or
`xdg-desktop-portal-kde`) is recommended for the folder picker.

Windows 10/11 builds use DirectX 12 (including software adapters) and the static
Microsoft C runtime. The executable is not code-signed, so Windows may show a
SmartScreen warning for the download.

## Use

- **Open folder** selects the directory shown in the left-hand tree. Expand
  folders on demand; **Refresh** reloads the tree.
- Click a Markdown file to read it. Files open in tabs; opening the same file
  again selects its existing tab without losing edits.
- Web links open in your default browser. Relative and absolute local links
  open in another tab. UTF-8 text files are shown as plain text; PDFs, images,
  and other binary documents use the system's default application. Executables
  and shortcuts are not launched.
- **Edit** changes the current tab into a plain-text editor. **Save** writes
  the changes and returns to the preview; **Discard** returns without writing.
  Save and Discard are only visible while editing.
- **Options** contains only light/dark mode, font (sans serif or monospace),
  and font size. Appearance and the last selected folder are remembered.

Tabs with unsaved changes show `*`. Closing a dirty tab or quitting prompts
to save, discard, or cancel. Changing the tree's folder keeps existing tabs.

| Shortcut | Action |
| --- | --- |
| Ctrl+O | Open folder |
| Ctrl+E | Edit current file |
| Ctrl+S | Save and return to preview |
| Ctrl+W | Close current tab |

You can also pass a file or folder at startup:

```text
mdv "path to a file.md"
mdv "path to a folder"
```

Try `examples/welcome.md` for a short rendering and link-navigation demo.

## Markdown and files

Headings, emphasis, lists, task lists, tables, block quotes, fenced code,
links, and local/HTTP(S) images are supported. Standard heading links such as
`#my-heading` and explicit heading IDs (`## Title {#custom-id}`) work, including
fragments on links to other files. Relative images resolve from the document's
folder, not the application's working directory.

HTML is displayed as text, not executed. There is no browser engine, plugin
system, Mermaid renderer, or LaTeX engine. Remote images may make network
requests when a document is viewed.

Editing supports UTF-8 files, including UTF-8 BOMs. Saves preserve BOMs and
LF/CRLF line-ending style and replace the file using a same-directory
temporary file rather than truncating it in place. A file changed externally
since editing began is not overwritten; the draft remains available to copy.
Clicking Edit reloads the latest disk contents. Discard returns to the
previously loaded preview.

## Build and test

Install the stable Rust toolchain (Rust 1.95 or newer). Windows also needs
Visual Studio C++ build tools. On Debian/Ubuntu, install the build dependencies:

```sh
sudo apt install build-essential pkg-config libx11-dev libxcursor-dev libxi-dev \
  libxrandr-dev libxkbcommon-dev libwayland-dev libgl1-mesa-dev \
  libegl1-mesa-dev libdbus-1-dev
```

```sh
cargo run -- examples/welcome.md
cargo fmt --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked
cargo build --release --locked
```

The release executable is in `target/release`. GitHub Actions performs these
checks, starts the native application on both platforms, and builds downloadable
x86_64 archives. A `v*` tag publishes the checked artifacts as a GitHub release.
Archives include dependency license notices and SHA-256 checksums.

## License

MDV is licensed under [MIT](LICENSE). Dependencies and bundled fonts retain
their own licenses, included in `THIRD-PARTY-LICENSES.txt` in release archives.
