# 🤖 GitHub Copilot CLI Token Insights Dashboard

[繁體中文版 (Traditional Chinese Version)](./README.md)

This is a local token consumption and session analysis dashboard designed specifically for **GitHub Copilot CLI**. Powered by a high-performance **Rust (Axum)** backend and a beautiful **dark glassmorphic (Glassmorphism)** frontend, it helps you easily monitor daily token cache hit rates, reasoning token consumption, and **reconstruct/restore the historical conversation timeline of every session**!

---

## 🌟 Features

This dashboard provides comprehensive local data visualization, featuring four core capabilities:

### 1. 📊 Daily Real-time Dashboard
- **Live Metrics Aggregation**: At-a-glance metrics for daily total token consumption, input/output token ratio, cache read tokens, and reasoning tokens.
- **Token Trend & Cache Chart**: Uses Chart.js to render daily session token consumption, cache hit rate, and session turns in smooth, interactive charts.
- **🔴 Live Monitor (Auto-refresh)**: Enable auto-refresh with customizable intervals (5s, 10s, 30s) and a smooth countdown progress bar. Syncs live with terminal conversations.

### 2. 📅 Monthly Aggregation
- **Monthly Trend Chart**: Displays daily token usage and session count trends across the month.
- **🏢 Most Active Project Directories**: Tracks sessions and tokens by working directory (CWD), showing where AI assistance was used the most.
- **🤖 Model Breakdown**: Lists session counts and token ratios for different LLMs (e.g. GPT-4o, Claude 3.5 Sonnet, etc.) used during the month.

### 3. 🔍 Interactive Session History
- **Rich Columns**: Lists sessions with columns like name/ID, model, max turns, input/output/cache/total tokens, and total API duration (ms).
- **Flexible Sorting**: Click headers to sort ascending/descending to find high-consuming or frequent sessions.

### 4. ⏱️ Session Timeline Drawer
- **Slide-out Drawer**: Clicking a session slides out a historical conversation timeline drawer.
- **Conversation Reconstruction**:
  - **User Prompt**: Displays user bubbles with attachments/context badges.
  - **Agent Reply**: Shows the AI's reasoning steps and markdown rendering of agent responses with syntax highlighting.
  - **Tool Step**: Automatically expands tool execution details showing tool name, arguments, exit code, stdout, and stderr.

---

## 🚀 Setup & Launch

All data is parsed and rendered entirely on your local machine, ensuring absolute data privacy and security. Follow these steps to configure and launch:

### Phase 1: Data Collection Configuration

The dashboard visualizes data streamed from the **GitHub Copilot CLI** status line to a local collection script.

#### 1️⃣ Install Rust Environment
In your terminal (Linux / WSL2 / macOS), execute the official installer:
```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
```
> [!IMPORTANT]
> **Installation Prompt**: When prompted with the installation menu, simply press **`Enter`** to select `1) Proceed with standard installation (default)`.

After installation, load Rust environment variables:
```bash
source "$HOME/.cargo/env"
```
*(Verify installation: Run `cargo --version`. It should display the installed cargo version.)*

#### 2️⃣ Deploy the Data Collection Script
Create a local config directory, copy the script from the project to `~/.copilot`, and grant execution permissions:
```bash
# Create directory and copy script
mkdir -p ~/.copilot && cp shell/statusline-token.sh ~/.copilot/statusline-token.sh

# Ensure script is executable
chmod +x ~/.copilot/statusline-token.sh
```

#### 3️⃣ Edit Copilot CLI Settings
Open or create the configuration file with your editor:
```bash
nano ~/.copilot/settings.json
```
For a **brand new configuration**, paste the following:
```json
{
  "statusLine": {
    "type": "command",
    "command": "/home/<username>/.copilot/statusline-token.sh",
    "padding": 1
  }
}
```
If you **already have existing configurations**, merge the `statusLine` block into it:
```json
{
  "footer": {
    "showDirectory": true,
    "showBranch": true
  },
  "statusLine": {
    "type": "command",
    "command": "/home/<username>/.copilot/statusline-token.sh",
    "padding": 1
  }
}
```
> [!NOTE]
> **Home Directory Hint**: Replace `/home/<username>` with your actual home directory path (run `echo $HOME` in terminal to check it).

#### 4️⃣ Restart & Verify
1. **Restart Copilot CLI**: Exit the current Copilot CLI session and re-enter to apply the new settings.
2. **Verify Status Line**: Upon starting a session, you should see a beautifully rendered status line at the bottom:
   ```text
   🤖 Auto → GPT-4o • #1 • ↑ 22.8k • c 22.0k/0 • ↓ 61 • r 29 • total 22.9k • +22.9k • last 22.8k/61 • ctx 9%
   ```
3. **Troubleshooting Tools**:
   - standalone script test:
     ```bash
     echo '{}' | ~/.copilot/statusline-token.sh
     ```
   - JSON validation test:
     ```bash
     jq . ~/.copilot/settings.json
     ```

---

### Phase 2: Start the Dashboard

Navigate to the project root directory and run:

```bash
cargo run
```
> [!NOTE]
> During the first launch, Cargo will download dependencies and compile the binary locally (takes 1~2 minutes). Subsequent starts take less than 1 second offline.

Once the terminal displays the success message:
```text
🚀 GitHub Copilot CLI Token Insights Dashboard is running on: http://localhost:3000
```
Open [**`http://localhost:3000`**](http://localhost:3000) in your browser to start exploring!

---

### Phase 3: Run as a Background Service (systemd)

To run the dashboard as a background service without keeping a terminal open, we recommend using user-level `systemd` services:

#### 1️⃣ Compile a Release Build
For optimal performance and minimal memory usage, compile a release binary:
```bash
cargo build --release
```

#### 2️⃣ Configure systemd Service
We have provided a service file template. Register it with your system using the following commands:
```bash
# Create systemd user configuration directory
mkdir -p ~/.config/systemd/user/

# Replace the template directory path and copy to systemd directory
sed "s|<PROJECT_DIR>|$PWD|g" shell/copilot-insights.service > ~/.config/systemd/user/copilot-insights.service

# Reload systemd configuration
systemctl --user daemon-reload
```

#### 3️⃣ Start & Enable the Service
```bash
# Start the service
systemctl --user start copilot-insights.service

# Enable the service to start on boot
systemctl --user enable copilot-insights.service
```

> [!TIP]
> **Enable Background Linger**:
> User-level services stop when you log out of your SSH/terminal session. To run them persistently in the background, enable `linger` on your system:
> ```bash
> sudo loginctl enable-linger $USER
> ```

#### 4️⃣ Service Management Commands
* **Check service status**: `systemctl --user status copilot-insights.service`
* **Check live logs**: `journalctl --user -u copilot-insights.service -n 50 -f`
* **Restart service**: `systemctl --user restart copilot-insights.service`
* **Stop service**: `systemctl --user stop copilot-insights.service`

---

### Phase 4: Advanced Configurations

#### Customizing Data Paths
The dashboard automatically searches for your default `~/.copilot` folder. If your Copilot CLI data is saved at a custom location, specify it using the `COPILOT_DIR` environment variable before running:

```bash
export COPILOT_DIR="/your/custom/path/.copilot"
cargo run
```

---

## ❓ FAQ

### Q: Why do individual conversation turns in the session drawer only show "Out" and "Total", but no "In" (Input) tokens?

**A:** This is not a dashboard limitation, but rather a **design choice of GitHub Copilot CLI's local event logging**:
1. **Turn Event Limits**: In the Copilot CLI local `events.jsonl` files, the `assistant.message` event logs only record the **`outputTokens`** produced by that specific reply. They do not record `inputTokens` or cache stats for that individual turn.
2. **Session Aggregated Summary**: Copilot CLI only aggregates and records the total input tokens, cache read/write tokens, and reasoning tokens of the entire conversation when you **end the session** (i.e. writing the final `session.shutdown` event).
3. **Frontend Fallback**: To display total tokens per turn, the frontend calculates `inTokens + outTokens` as a fallback. Since `inTokens` is absent (parsed as 0), the `Total` matches the `Out` value, and the empty `In` badge is automatically hidden to keep the interface clean.
