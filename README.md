# 🤖 GitHub Copilot CLI Insights Dashboard

[English Version (英文版)](./README.en.md)

這是一個專門為 **GitHub Copilot CLI** 設計的本地 Token 消耗與會話分析看板。使用高效能的 **Rust (Axum)** 作為後端，搭配 **深色毛玻璃風格 (Glassmorphism)** 前端，協助您輕鬆查看每日的 Token 快取命中率、推理 Token 消耗，並能**重建與還原每個會話 (Session) 的歷史對話時間軸**！

---

## 🌟 功能說明 (Features)

本看板提供全方位的本地數據可視化，包含以下四大核心功能：

### 1. 📊 每日即時分析看板 (Daily Real-time Dashboard)
- **即時指標彙整**：一目了然每日的 Token 總消耗、輸入/輸出 Token 佔比、快取讀取 Token 以及推理 Token 的使用量。
- **Token 趨勢與快取圖表**：使用 Chart.js 以平滑曲線呈現每日各 Session 的 Token 消耗波動、快取命中率與對話 Turn 數對比。
- **🔴 即時自動更新機制 (Live Monitor)**：支援一鍵開啟自動刷新，可自訂 5 秒、10 秒或 30 秒的更新頻率。當您在終端機中與 Copilot CLI 對話時，看板數據將會即時同步，並附有倒數計時進度條。

### 2. 📅 月度數據彙整 (Monthly Aggregation)
- **月度趨勢圖表**：折線圖展示單月內每日的 Token 總體使用情況與會話數的趨勢變化。
- **🏢 最常活動的專案目錄**：統計您在不同專案工作目錄（CWD）下的 Copilot 會話次數與 Token 消耗，方便追蹤哪些專案投入了最多 AI 輔助。
- **🤖 使用的模型佔比**：清晰列出不同 LLM（如 GPT-4o, Claude 3.5 Sonnet 等）在當月的會話次數與 Token 佔比。

### 3. 🔍 互動式會話歷史清單 (Interactive Session History)
- **多維度欄位**：以表格形式完整列出歷史會話。欄位包含會話名稱/ID、使用的模型、最大 Turn 數、輸入/輸出/快取 Token 以及 API 總耗時（毫秒）。
- **靈活排序**：點選任一欄位標頭即可進行即時升冪或降冪排序，幫助您快速篩選出高消耗或高頻次的會話。

### 4. ⏱️ 精準會話時間軸還原 (Session Timeline Drawer)
- **側邊滑出式抽屜**：點擊列表中的會話，右側將流暢滑出詳細的歷史對話時間軸。
- **對話內容重建**：
  - **使用者提示詞 (User Prompt)**：清晰的對話泡泡，並標示附加的 context 狀態。
  - **助理思考與回覆 (Agent Reply)**：呈現 LLM 的思維過程（Reasoning Process）與 Markdown 排版渲染的代碼高亮。
  - **工具呼叫步驟 (CLI Tool Step)**：自動展開 Copilot CLI 呼叫的本地 CLI 工具名稱、入參（Arguments）、環境 context、執行狀態碼（Exit Code）以及標準輸出（Stdout）與錯誤輸出（Stderr），徹底還原 AI 在您電腦上的操作路徑。

---

## 🚀 配置與啟動指南 (Setup & Launch)

本專案完全運行於您的本地端，確保所有數據的隱私與安全性。請按照以下步驟完成配置與啟動：

### 一、前置配置 (Copilot CLI 數據收集設定)

本看板分析的數據來自於 **GitHub Copilot CLI** 內建狀態列（Status Line）傳送給本地收集腳本的 JSON 串流。

#### 1️⃣ 安裝 Rust 環境
在您的終端機 (Linux / WSL2 / macOS) 中執行官方安裝腳本：
```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
```
> [!IMPORTANT]
> **安裝選項提示**：當畫面停在安裝選單時，直接**按下 `Enter`** 選擇預設的 `1) Proceed with standard installation (default)` 即可。

安裝完成後，載入 Rust 環境變數：
```bash
source "$HOME/.cargo/env"
```
*(驗證安裝：輸入 `cargo --version`，若正常顯示版本號即代表安裝成功。)*

#### 2️⃣ 設定數據收集腳本
建立本地配置目錄，將本專案中的收集腳本複製至 `~/.copilot` 目錄，並賦予執行權限：
```bash
# 建立目錄並複製腳本
mkdir -p ~/.copilot && cp shell/statusline-token.sh ~/.copilot/statusline-token.sh

# 確保腳本有執行權限
chmod +x ~/.copilot/statusline-token.sh
```

#### 3️⃣ 編輯 Copilot CLI 設定檔
使用您的編輯器開啟或建立設定檔：
```bash
nano ~/.copilot/settings.json
```
若為**全新配置**，請寫入以下內容：
```json
{
  "statusLine": {
    "type": "command",
    "command": "/home/<username>/.copilot/statusline-token.sh",
    "padding": 1
  }
}
```
若**已有其他設定**，請將 `statusLine` 區塊合併加入，例如：
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
> **家目錄路徑提示**：請將 `/home/<username>` 替換為您的實際家目錄路徑（可在終端機執行 `echo $HOME` 來查詢）。

#### 4️⃣ 重新啟動與驗證
1. **重啟 Copilot CLI**：退出目前的 Copilot CLI 會話並重新開啟以套用新設定。
2. **狀態列檢查**：進入 Copilot CLI 會話後，您應該會在最底端看到類似以下的 Token 狀態列：
   ```text
   🤖 Auto → GPT-4o • #1 • ↑ 22.8k • c 22.0k/0 • ↓ 61 • r 29 • total 22.9k • +22.9k • last 22.8k/61 • ctx 9%
   ```
3. **除錯工具**：
   - 測試腳本是否可正常執行：
     ```bash
     echo '{}' | ~/.copilot/statusline-token.sh
     ```
   - 確認 `settings.json` 是否為合法 JSON 格式：
     ```bash
     jq . ~/.copilot/settings.json
     ```

---

### 二、啟動看板服務

切換至專案根目錄，執行以下命令：

```bash
cargo run
```
> [!NOTE]
> 初次執行時，Rust 會自動下載需要的依賴庫並進行本地編譯（約需 1~2 分鐘，後續啟動僅需 1 秒且無需網路）。

當終端機顯示以下成功訊息時：
```text
🚀 GitHub Copilot CLI Insights Dashboard is running on: http://localhost:3000
```
請在瀏覽器中打開 [**`http://localhost:3000`**](http://localhost:3000)，即可開始使用您的看板！

---

### 三、設定為常駐背景服務 (systemd)

如果您希望將看板作為本地常駐服務運作（免去每次手動開啟終端機執行 `cargo run`），推薦使用 Linux 原生的 `systemd` 使用者級別服務：

#### 1️⃣ 編譯發行版本 (Release Build)
為求最佳效能與資源使用效率，請先編譯獨立的 Release 二進位檔：
```bash
cargo build --release
```

#### 2️⃣ 配置 systemd 服務
本專案已為您準備好服務描述檔範本，您只需執行以下指令即可將其複製並註冊至系統中：
```bash
# 建立 systemd 使用者配置目錄
mkdir -p ~/.config/systemd/user/

# 替換範本中的專案路徑並複製到 systemd 目錄中
sed "s|<PROJECT_DIR>|$PWD|g" shell/copilot-insights.service > ~/.config/systemd/user/copilot-insights.service

# 重新載入設定
systemctl --user daemon-reload
```

#### 3️⃣ 啟動與管理服務
```bash
# 啟動服務
systemctl --user start copilot-insights.service

# 設定開機自動啟動
systemctl --user enable copilot-insights.service
```

> [!TIP]
> **常駐背景執行提示 (Linger)**：
> 使用者級別服務預設會在您登出 SSH/終端機時停止。若要讓服務在背景永久常駐，請在您的主機上執行以下指令來啟用 `linger`：
> ```bash
> sudo loginctl enable-linger $USER
> ```

#### 4️⃣ 常用管理命令
* **查看服務狀態**：`systemctl --user status copilot-insights.service`
* **查看即時日誌**：`journalctl --user -u copilot-insights.service -n 50 -f`
* **重啟服務**：`systemctl --user restart copilot-insights.service`
* **停止服務**：`systemctl --user stop copilot-insights.service`

---

### 四、進階配置與環境變數

#### 自訂 `.copilot` 數據路徑
本專案預設會自動尋找您目前的 `~/.copilot` 資料夾。如果您的 Copilot CLI 數據儲存於其他自訂路徑，您可以透過設定 `COPILOT_DIR` 環境變數來指定路徑，然後啟動服務：

```bash
export COPILOT_DIR="/your/custom/path/.copilot"
cargo run
```

---

## ❓ 常見問答 (FAQ)

### Q: 為什麼在會話明細的對話時間軸中，每一筆對話明細只有出現 `Out`（輸出）與 `Total`（總計）？那 `In`（輸入）呢？

**A:** 這並非看板程式的限制，而是 **GitHub Copilot CLI 本地原始日誌的設計限制** 所導致：
1. **單筆回覆事件限制**：在 Copilot CLI 本地生成的 `events.jsonl` 日誌檔中，每次對話的 `assistant.message` 事件資料內**僅記錄了 `outputTokens`**（該次回覆生成的 Token 數），並沒有記錄該次對話傳送的 `inputTokens`（輸入 Token 數）或快取讀取 Token 數。
2. **總量聚合統計**：Copilot CLI 只有在您**結束整個會話**（即寫入最後一個 `session.shutdown` 事件）時，才會一次性在會話總帳中累計並記錄整場會話的總輸入 Token 數與快取統計數。
3. **前端 Fallback 邏輯**：看板網頁為了呈現資訊，在前端計算總數時會以 `inTokens + outTokens` 作為 Fallback 補齊。由於該單次對話無 `inTokens` 資料（解析為 0），因此繪製出來的 `Total`（總計）會剛好與 `Out`（輸出）完全一致，且 `In` 標籤會被自動隱藏。
