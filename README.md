# Wflow

**High-performance native AI voice dictation, written in Rust.**

Wflow captures your microphone while you hold a global push-to-talk hotkey,
streams the audio to the speech-to-text provider of your choice, and pastes the
transcript straight into whatever text box was focused. A single native
executable replaces the 400 MB Electron client — typical idle memory is around
12 MB.

> **Unofficial project.** Wflow is not affiliated with, endorsed by, or
> connected to Wispr Flow, Baseten, Groq or ElevenLabs. You supply your own
> account and API keys and are responsible for complying with each provider's
> terms of service.

---

## ✨ Features

### Speech-to-text engines (switchable in Settings)

| Provider | Transport | Notes |
|---|---|---|
| **Wispr Flow** | Bidirectional gRPC (Protobuf) over HTTP/2 | Lowest latency streaming, needs your account JWT |
| **Groq Whisper** | HTTPS multipart upload | Whisper v3 Turbo on Groq LPU, ~200 ms per utterance |
| **ElevenLabs Scribe** | HTTPS multipart upload | State-of-the-art multilingual accuracy |

### Dictation

- **Global push-to-talk hotkey** with full modifier combinations (`Ctrl+Space`,
  `Alt+Space`, `Ctrl+Shift+D`, …) — captured by a low-level `WH_KEYBOARD_LL`
  hook on Windows and by an X11/XWayland keyboard watcher on Linux.
- **Tap-to-toggle mode** as an alternative to push-to-talk.
- **Floating HUD overlay** with a live microphone waveform, provider badge and
  status colour (red = listening, purple = transcribing, green = inserted).
  Native topmost window on Windows, transparent egui viewport elsewhere.
- **Context-aware dictation** — the active application and surrounding text can
  be sent to the provider (opt-in, see *Privacy*).
- **Thai typography normalisation** — combining vowels and tone marks are
  repositioned so Thai renders correctly in the GUI and in pasted text.
- **Auto-paste** through the clipboard plus synthetic `Ctrl+V`, with a
  configurable safety limit on recording length.

### 🎚️ Voice quality

- **Silence filter (noise gate)** — the ambient noise floor is measured while
  you dictate and only chunks that are clearly speech are sent to the
  provider. A short pre-roll buffer keeps the first syllable intact, and the
  recording can stop automatically after a long silence. The studio shows a
  live voice-activity indicator.
- **Hallucination guard** — speech models happily invent "Thank you.",
  "(upbeat music)" or repeat one word fifty times when they are fed noise.
  Every transcript is validated before it is inserted, so model garbage never
  ends up in your editor.
- **Per-language spell check** — an offline dictionary fixes the common
  misspellings of each language, and the language model can take it further
  when a Groq key is configured.
- **Mixed Thai/Latin handling** — `ทดสอบAPI` becomes `ทดสอบ API`, and each word
  is spell-checked with the dictionary of its own script.

### AI post-processing (optional)

Raw transcripts can be polished by Groq's `llama-3.1-8b-instant` with one of
six styles: `smart_cleanup`, `casual`, `professional`, `code`, `bullet_points`
or `raw`.

### Local data

- **SQLite history** in the Wispr Flow schema — searchable, copyable, with
  per-entry deletion and a one-click "clear everything" action.
- **Personal vocabulary and snippets** — words are sent to the provider as
  vocabulary hints, and trigger phrases can be expanded automatically
  (`brb` → `be right back`).
- **Single-instance guard** so a second launch cannot duplicate the keyboard
  hook or the tray icon.

### Desktop integration

- **System tray** (Windows notification area / StatusNotifierItem on Linux) with
  open, hide, start/stop dictation and quit actions.
- **Auto-start** at login (Windows registry `Run` key / XDG autostart entry).
- **Diagnostics** — rotating log files plus a panic hook, so a crash is
  reproducible by the user instead of vanishing.
- **In-app update checker** — a dedicated **update button in the top bar**
  (visible on every tab), a clickable version indicator in the sidebar and a
  *Check for updates* entry in the system tray. It queries the GitHub release
  feed, shows the new version with release notes, and links to the right
  installer for your platform. Checks also run in the background every 15
  minutes while the app is open.

---

## 📦 Installation

### Windows

Download `Wflow-Setup-<version>.exe` from the
[releases page](https://github.com/redpotion39/wflow/releases) and run it.
The installer registers `wflow.exe`, creates Start Menu and Desktop shortcuts
and can enable auto-start.

To build it yourself you need [Inno Setup 6](https://jrsoftware.org/isinfo.php):

```powershell
cargo build --release
iscc installer.iss
```

### Linux

**Debian / Ubuntu**

```bash
sudo dpkg -i wflow_<version>_amd64.deb
sudo apt-get install -f   # install missing runtime dependencies
```

**Universal self-extracting installer** (any distribution)

```bash
./wflow-<version>-linux-x86_64.run --user     # no root required
sudo ./wflow-<version>-linux-x86_64.run --system
```

**Install script**

```bash
./install.sh --user
sudo ./install.sh --system
```

**Build all packages from source**

```bash
sudo apt-get install -y build-essential pkg-config libasound2-dev libssl-dev \
    libx11-dev libxkbcommon-dev libxcursor-dev libxi-dev libxrandr-dev \
    libgl1-mesa-dev xdotool dpkg-dev

bash packaging/linux/build-all.sh
```

Outputs land in `dist/`: a `.deb`, a `.run` installer and a `.tar.gz` bundle.

### Requirements

* Windows 10/11 x64, or a 64-bit Linux distribution
* A Rust toolchain (stable) if you want to build from source
* On Linux: `xdotool` for pasting, and an audio player (`paplay`, `pw-play`,
  `aplay` or `ffplay`) for the earcons

---

## ⌨️ Usage

1. Focus any text box (Notepad, Chrome, VS Code, Word, Discord, …).
2. Hold the dictation hotkey — `F8` by default, or whatever you configured.
3. Speak. The HUD shows a live waveform while recording.
4. Release. The transcript is transcribed, optionally cleaned up by the LLM, and
   pasted into the focused field.
5. The entry is written to the local history.

Keyboard shortcuts:

| Action | Default |
|---|---|
| Push-to-talk dictation | `F8` |
| Show / hide the window | `F9` |
| Check for updates | the `🔄 อัปเดต` button in the top bar, or `wflow --check-updates` |

---

## ⚙️ Configuration

Configuration lives in a per-user directory:

| Platform | Configuration | Data | Logs |
|---|---|---|---|
| Windows | `%APPDATA%\Wflow\` | `%APPDATA%\Wflow\` | `%APPDATA%\Wflow\logs\` |
| Linux | `~/.config/wflow/` | `~/.local/share/wflow/` | `~/.local/state/wflow/logs/` |

`config.json` holds non-secret preferences. **API keys and session tokens are
never written there** — they live in `secrets.json`, created with `0600`
permissions and replaced atomically. See [SECURITY.md](SECURITY.md).

Secrets can also be provided per-process, which takes precedence:

```bash
export WFLOW_AUTH_TOKEN="..."
export WFLOW_GROQ_API_KEY="gsk_..."
export WFLOW_ELEVENLABS_API_KEY="xi-..."
export WFLOW_BASETEN_API_KEY="Api-Key ..."
export WFLOW_UPDATE_REPO="owner/repo"     # override the update feed
export WFLOW_GITHUB_TOKEN="ghp_..."        # only needed for a private repo
```

The update checker reads the public GitHub Releases feed. If the repository is
private, export `WFLOW_GITHUB_TOKEN` with a token that has `read:packages`
/ public-repository read access.

The gRPC gateway key is **not** embedded in the binary. If you use the Wispr
Flow provider, enter your key in **Settings → STT Provider**, or export
`WFLOW_BASETEN_API_KEY`.

---

## 🔒 Privacy

Wflow is designed so you control what leaves your machine:

* **Context sharing is opt-out and visible.** Surrounding text is only sent to
  the provider when *Settings → Privacy & Local Data → Share surrounding text
  context* is enabled. Turn it off to dictate sensitive content.
* **Local history retention is opt-in.** The contents of the focused text box
  are *not* stored by default.
* **Erase anytime** with *Settings → Privacy & Local Data → Clear history*, or
  delete `flow.sqlite`.
* **Logs contain no credentials and no transcripts** — only diagnostics.

---

## 🏗️ Project structure

```
wflow/
├── proto/flow_api.proto     # Reconstructed Protobuf definition
├── src/
│   ├── main.rs              # CLI, logging, background coordinator
│   ├── lib.rs               # Library root
│   ├── audio.rs             # cpal capture, 16 kHz resampling, RMS meter
│   ├── config.rs            # AppConfig + KeyCombo
│   ├── db.rs                # SQLite history, dictionary, snippets
│   ├── grpc.rs              # Tonic gRPC streaming client
│   ├── gui.rs               # egui studio, history, dictionary, settings
│   ├── http.rs              # Shared HTTP client + retry policy
│   ├── instance.rs          # Single-instance lock
│   ├── lang.rs              # Script detection & mixed-language spacing
│   ├── paths.rs             # Per-user directories
│   ├── quality.rs           # Hallucination / model-garbage guard
│   ├── proto.rs             # prost messages for flow_api.v1
│   ├── secrets.rs           # 0600 secrets file
│   ├── sound.rs             # Synthesised earcons
│   ├── spell.rs             # Per-language offline spell dictionary
│   ├── stt.rs               # Groq / ElevenLabs HTTP clients + LLM cleanup
│   ├── text.rs              # Shared text helpers
│   ├── thai.rs              # Thai typography normalisation
│   ├── ui.rs                # Dictation status definitions
│   ├── vad.rs               # Voice activity detection (silence filter)
│   ├── update.rs            # GitHub release update checker
│   └── platform/
│       ├── mod.rs           # Cross-platform interface
│       ├── windows.rs       # Win32 hook, UI Automation, SendInput, HUD
│       └── linux.rs         # X11 hotkeys, ksni tray, paste, autostart
├── packaging/linux/         # .deb / .run builders, desktop + AppStream data
├── .github/workflows/       # CI, release and package builds
├── LICENSE                  # MIT
└── SECURITY.md
```

---

## 🧪 Development

```bash
cargo build --release      # optimised binary
cargo test                 # unit tests
cargo clippy --all-targets -- -D warnings
cargo fmt --all
```

CI runs formatting, Clippy, the test suite, a build matrix (Linux / Windows /
macOS), `cargo audit` and a repository secret-leak scan on every push.

---

## 📄 License

MIT — see [LICENSE](LICENSE).

Third-party services keep their own licenses and terms; Wflow is an
independent client and ships no third-party credentials.
