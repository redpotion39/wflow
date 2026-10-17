# Wispr Flow Rust (`wispr-flow-rust`)

A native, lightweight, ultra-fast Rust rewrite and client for Wispr Flow dictation, reverse-engineered using **MCP REA**.

Replaces the resource-heavy Electron (~400MB RAM) and .NET NativeAOT multi-process pipeline with a **single, zero-runtime native Rust executable (~12MB RAM)**.

---

## ⚡ Key Features

- **Multi-Provider Speech-to-Text (STT) AI Engines**:
  - **⚡ Wispr Flow Cloud**: Native bidirectional gRPC Protobuf streaming over HTTP/2.
  - **🚀 Groq Whisper (v3 Turbo)**: Ultra-low latency cloud STT via Groq's high-throughput LPU API.
  - **🎙️ ElevenLabs Scribe STT**: State-of-the-art multilingual speech recognition.
  - Switch providers with 1 click in Settings tab with automatic API key & model management.
- **Floating Overlay HUD & Waveform Indicator**:
  - Lightweight native Windows Topmost floating pill that automatically appears at the bottom-center of the screen during dictation.
  - Dynamic status dot (Pulsing Red for Listening, Purple for Transcribing, Green for Done/Inserted).
  - Provider badge displaying active AI service (`Wispr`, `Groq`, `11Labs`).
  - Real-time double-buffered 7-bar audio waveform animating directly to microphone volume.
  - Smoothly auto-dismisses after dictation finishes.
- **Modern Sleek Desktop GUI & System Tray**:
  - High-performance 60 FPS hardware-accelerated GUI built with `egui` / `eframe`.
  - **Taskbar System Tray Integration**: Lives unobtrusively in the Windows Notification Area. Closing the window minimizes it to tray.
  - **Global Window Toggle (`F9`)**: Press `F9` anywhere in Windows to instantly show or hide the Wispr Flow Studio.
  - **Customizable Global Hotkeys**: Interactive hotkey recording modal to set any key for dictation and window toggle.
  - **Tray Context Menu**: Left-click to toggle, right-click for quick actions ("Open", "Hide", "Exit").
  - **Full Thai Font Support**: Integrates native Windows Leelawadee UI & Tahoma fonts for crisp Thai rendering.
  - **Live Audio Studio**: 32-bar animated waveform visualizer reacting in real time to microphone volume.
  - **Context-Aware HUD**: Displays detected active app (`chrome.exe`, `code.exe`, `notepad.exe`) and surrounding text context.
  - **Interactive History**: Searchable card list of all past dictations with 1-click copy to clipboard.
  - **Settings Manager**: Customize push-to-talk hotkey, transcription language, cloud endpoints, and auto-paste behaviors.
- **Blazing Fast & Lightweight**: Uses ~12 MB of memory compared to >350 MB for the official Electron client.
- **Low-Level Native Windows Integration**:
  - Global low-level keyboard hook (`WH_KEYBOARD_LL`) for push-to-talk dictation anywhere in Windows.
  - Windows UI Automation (`IUIAutomation`) to capture current text box context (`before_text`, `after_text`, `process_name`).
  - Synthetic input pasting via `SendInput` and clipboard.
- **Real-Time 16kHz Audio Pipeline**:
  - Built with `cpal` directly interfacing with Windows WASAPI.
  - Dynamic sample-rate conversion to 16,000 Hz 16-bit mono PCM.
- **Local SQLite History & Dictionary**:
  - Automatically stores dictation history and vocabulary matching the original `flow.sqlite` schema.

---

## 📁 Project Structure

```
D:\project\wme
├── proto/
│   └── flow_api.proto         # Reconstructed Protobuf definition
├── src/
│   ├── proto.rs               # Prost message definitions matching flow_api.v1
│   ├── grpc.rs                # Tonic gRPC streaming client
│   ├── audio.rs               # cpal 16kHz audio capture & RMS meter
│   ├── gui.rs                 # Modern egui GUI (Studio, History, Settings)
│   ├── platform/
│   │   ├── mod.rs             # Cross-platform interface
│   │   └── windows.rs         # Win32 keyboard hook, UI Automation & SendInput
│   ├── db.rs                  # SQLite history & dictionary manager
│   ├── config.rs              # JSON configuration
│   ├── ui.rs                  # Dictation status definitions
│   └── main.rs                # Multi-threaded async runtime orchestrator
├── Cargo.toml                 # Dependencies and build settings
└── README.md
```

---

## 🚀 Building & Running

### Prerequisites
- Windows 10 / 11 (x64)
- Rust toolchain (`rustup install stable`)

### Build Release Binary
```powershell
cargo build --release
```
The compiled binary will be placed at:
```
target\release\wispr-flow-rust.exe
```

### Run
```powershell
cargo run --release
```

---

## 🎙️ How to Use

1. Click on any text box or application where you want to type (e.g. Notepad, Chrome, VS Code, Discord, Word).
2. **Press and hold `F8`**: The console will show the real-time audio waveform:
   ```
   [🎙️  LISTENING] ██████████          (Release F8 to finish)
   ```
3. **Speak clearly into your microphone**: Audio packets are streamed in real time.
4. **Release `F8`**: The audio stream commits, the transcript is received from the server, and the text is automatically pasted into the active text box!
5. All dictations are recorded in the local SQLite database.

---

## 📦 Installation & Packaging

### Windows Installer (Inno Setup)
- **Installer executable**: `dist/Wflow-Setup-0.1.0.exe`
- Automatically installs `wflow.exe`, creates Start Menu shortcuts, Desktop icon, and optional startup integration.
- To recompile:
  ```powershell
  iscc installer.iss
  ```

### Linux (Ubuntu / Debian & All Distributions)

#### 1. Universal Self-Extracting Installer (`.run`):
Runs on any Linux distribution (Ubuntu, Debian, Fedora, Arch, etc.):
```bash
# User install (no sudo / root required):
./dist/wflow-0.1.0-linux-x86_64.run --user

# System-wide install:
sudo ./dist/wflow-0.1.0-linux-x86_64.run --system
```

#### 2. Install via Debian Package (`.deb`):
```bash
sudo dpkg -i dist/wflow_0.1.0_amd64.deb
sudo apt-get install -f   # Fix any missing system dependencies
```

#### 3. Install via Universal Shell Script:
```bash
# User install:
./install.sh --user

# System install:
sudo ./install.sh --system
```

#### 4. Build All Linux Packages from Source:
```bash
# Install Ubuntu build prerequisites
sudo apt-get update
sudo apt-get install -y build-essential pkg-config libasound2-dev libssl-dev libx11-dev libxkbcommon-dev libxcursor-dev libxi-dev libxrandr-dev libgl1-mesa-dev xdotool dpkg-dev

# Build all installers (.deb, .run, .tar.gz)
bash packaging/linux/build-all.sh
```
All outputs are generated in the `dist/` directory:
- `dist/wflow_0.1.0_amd64.deb`
- `dist/wflow-0.1.0-linux-x86_64.run`
- `dist/wflow-0.1.0-linux-x86_64.tar.gz`

#### 5. Automated GitHub Actions CI/CD:
Every push or release automatically triggers `.github/workflows/build-ubuntu.yml` to build all Linux installers (`.deb`, `.run`, `.tar.gz`) as downloadable artifacts.


---

## ⚙️ Configuration

A default `config.json` is automatically created on first run:
```json
{
  "api_endpoint": "https://api.wisprflow.ai:443",
  "auth_token": "Bearer YOUR_JWT_TOKEN",
  "language": "en",
  "hotkey": "F8",
  "db_path": "flow.sqlite"
}
```
You can update `auth_token` with your Wispr Flow session token (found in `%APPDATA%\Wispr Flow\flow.sqlite` or local storage) to connect to live services.
