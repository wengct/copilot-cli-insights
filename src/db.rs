use std::fs::{self, File};
use std::io::{Read, Seek, SeekFrom};
use std::time::SystemTime;
use rusqlite::{params, Connection};
use crate::{get_copilot_dir, parse_usage_entries};

/// 取得 SQLite 資料庫連接，資料庫存放於 ~/.copilot/copilot_cli_token_insights.db
pub fn get_db_conn() -> Result<Connection, String> {
    let copilot_dir = get_copilot_dir()?;
    let db_path = copilot_dir.join("copilot_cli_token_insights.db");
    Connection::open(&db_path).map_err(|e| format!("無法開啟資料庫: {}", e))
}

/// 初始化資料庫，建立資料表與必要的索引
pub fn init_db(conn: &Connection) -> Result<(), String> {
    // 建立 usage_entries 表，用於儲存 Token 請求紀錄
    conn.execute(
        "CREATE TABLE IF NOT EXISTS usage_entries (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            timestamp TEXT NOT NULL,
            date TEXT NOT NULL,
            session_id TEXT NOT NULL,
            session_name TEXT,
            transcript_path TEXT,
            cwd TEXT,
            version TEXT,
            turn_no INTEGER NOT NULL,
            model TEXT,
            model_id TEXT,
            
            -- Token 統計 (原始累計)
            tokens_input INTEGER,
            tokens_output INTEGER,
            tokens_cache_read INTEGER,
            tokens_reasoning INTEGER,
            tokens_total INTEGER,
            
            -- Delta Token 統計 (本次請求增量)
            delta_input INTEGER,
            delta_output INTEGER,
            delta_cache_read INTEGER,
            delta_reasoning INTEGER,
            delta_total INTEGER,
            
            -- 成本與時間
            duration_ms INTEGER,
            premium_requests INTEGER
        )",
        [],
    ).map_err(|e| format!("建立 usage_entries 表失敗: {}", e))?;

    // 建立唯一聯合約束，防止重複同步時寫入重複數據
    conn.execute(
        "CREATE UNIQUE INDEX IF NOT EXISTS uidx_session_turn ON usage_entries(session_id, turn_no)",
        [],
    ).map_err(|e| format!("建立唯一索引 uidx_session_turn 失敗: {}", e))?;

    // 建立日期索引以加速日明細與月報查詢
    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_usage_date ON usage_entries(date)",
        [],
    ).map_err(|e| format!("建立日期索引 idx_usage_date 失敗: {}", e))?;

    // 建立同步狀態記錄表
    conn.execute(
        "CREATE TABLE IF NOT EXISTS sync_state (
            filename TEXT PRIMARY KEY,
            last_synced_size INTEGER NOT NULL,
            last_synced_time INTEGER NOT NULL
        )",
        [],
    ).map_err(|e| format!("建立 sync_state 表失敗: {}", e))?;

    Ok(())
}

/// 增量同步使用量日誌檔到 SQLite 中
pub fn sync_usage_logs(conn: &Connection) -> Result<(), String> {
    let copilot_dir = get_copilot_dir()?;
    let usage_dir = copilot_dir.join("usage");
    if !usage_dir.exists() {
        return Ok(());
    }

    let entries = fs::read_dir(usage_dir).map_err(|e| format!("無法讀取 usage 目錄: {}", e))?;

    for entry in entries.flatten() {
        let file_type = match entry.file_type() {
            Ok(t) => t,
            Err(_) => continue,
        };

        if !file_type.is_file() {
            continue;
        }

        let filename = entry.file_name().to_string_lossy().into_owned();
        if !filename.starts_with("usage-") || !filename.ends_with(".jsonl") {
            continue;
        }

        // 解析檔名中的日期 YYYY-MM-DD
        let date_str = filename
            .trim_start_matches("usage-")
            .trim_end_matches(".jsonl")
            .to_string();

        let filepath = entry.path();

        // 查詢該檔案上一次同步時的大小 (Byte Offset)
        let last_synced_size: u64 = conn
            .query_row(
                "SELECT last_synced_size FROM sync_state WHERE filename = ?",
                params![filename],
                |row| row.get(0),
            )
            .unwrap_or(0u64);

        let mut file = File::open(&filepath).map_err(|e| format!("無法開啟日誌檔 {}: {}", filename, e))?;
        let metadata = file.metadata().map_err(|e| format!("無法取得檔案資訊 {}: {}", filename, e))?;
        let current_size = metadata.len();

        // 若檔案被截斷或重置，則從頭開始同步
        let start_pos = if current_size < last_synced_size {
            0
        } else {
            last_synced_size
        };

        // 有新資料寫入才進行同步
        if current_size > start_pos {
            file.seek(SeekFrom::Start(start_pos)).map_err(|e| format!("Seek 失敗 {}: {}", filename, e))?;
            let mut buffer = Vec::new();
            file.read_to_end(&mut buffer).map_err(|e| format!("讀取檔案失敗 {}: {}", filename, e))?;

            // 尋找最後一個完整的換行符，避免解析到寫入一半的行
            let mut read_len = buffer.len();
            while read_len > 0 && buffer[read_len - 1] != b'\n' {
                read_len -= 1;
            }

            if read_len > 0 {
                let new_content = String::from_utf8_lossy(&buffer[..read_len]);
                let parsed_entries = parse_usage_entries(&new_content);

                if parsed_entries.is_empty() {
                    continue;
                }

                // 啟動手動交易 (Transaction) 進行批次寫入以追求極致效能
                conn.execute("BEGIN TRANSACTION", []).map_err(|e| format!("Transaction BEGIN 失敗: {}", e))?;

                let mut success = true;
                for entry in &parsed_entries {
                    let tokens = entry.tokens.as_ref();
                    let delta = entry.delta_tokens.as_ref();
                    let cost = entry.cost.as_ref();

                    // 使用 INSERT OR IGNORE 確保若 turn 已存在則略過不重複插入
                    let insert_res = conn.execute(
                        "INSERT OR IGNORE INTO usage_entries (
                            timestamp, date, session_id, session_name, transcript_path, cwd, version, turn_no, model, model_id,
                            tokens_input, tokens_output, tokens_cache_read, tokens_reasoning, tokens_total,
                            delta_input, delta_output, delta_cache_read, delta_reasoning, delta_total,
                            duration_ms, premium_requests
                        ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
                        params![
                            entry.timestamp,
                            date_str,
                            entry.session_id,
                            entry.session_name.as_deref(),
                            entry.transcript_path.as_deref(),
                            entry.cwd.as_deref(),
                            entry.version.as_deref(),
                            entry.turn_no as i64,
                            entry.model.as_deref(),
                            entry.model_id.as_deref(),
                            tokens.map(|t| t.input as i64),
                            tokens.map(|t| t.output as i64),
                            tokens.and_then(|t| t.cache_read.map(|v| v as i64)),
                            tokens.and_then(|t| t.reasoning.map(|v| v as i64)),
                            tokens.map(|t| t.total as i64),
                            delta.map(|t| t.input as i64),
                            delta.map(|t| t.output as i64),
                            delta.and_then(|t| t.cache_read.map(|v| v as i64)),
                            delta.and_then(|t| t.reasoning.map(|v| v as i64)),
                            delta.map(|t| t.total as i64),
                            cost.and_then(|c| c.total_api_duration_ms.map(|d| d as i64)),
                            cost.and_then(|c| c.total_premium_requests.map(|r| r as i64))
                        ],
                    );

                    if let Err(e) = insert_res {
                        eprintln!("寫入資料庫失敗: {}", e);
                        success = false;
                        break;
                    }
                }

                if success {
                    let now = SystemTime::now()
                        .duration_since(SystemTime::UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_secs() as i64;

                    // 記錄本次同步完畢後的檔案大小位置與時間
                    let update_state_res = conn.execute(
                        "INSERT OR REPLACE INTO sync_state (filename, last_synced_size, last_synced_time) VALUES (?, ?, ?)",
                        params![filename, (start_pos + read_len as u64) as i64, now],
                    );

                    if update_state_res.is_ok() {
                        if let Err(e) = conn.execute("COMMIT TRANSACTION", []) {
                            eprintln!("Transaction COMMIT 失敗: {}", e);
                            let _ = conn.execute("ROLLBACK TRANSACTION", []);
                        }
                    } else {
                        let _ = conn.execute("ROLLBACK TRANSACTION", []);
                    }
                } else {
                    let _ = conn.execute("ROLLBACK TRANSACTION", []);
                }
            }
        }
    }

    Ok(())
}
