use axum::{
    extract::Path,
    http::StatusCode,
    response::IntoResponse,
    routing::get,
    Json, Router,
};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    fs::File,
    io::{BufRead, BufReader},
    path::PathBuf,
};
use tower_http::cors::CorsLayer;
use tower_http::services::ServeDir;

use rusqlite::params;
mod db;

#[tokio::main]
async fn main() {
    // 初始化 SQLite 資料庫並進行第一次增量同步
    if let Ok(conn) = db::get_db_conn() {
        if let Err(e) = db::init_db(&conn) {
            eprintln!("❌ 初始化 SQLite 資料庫失敗: {}", e);
        } else if let Err(e) = db::sync_usage_logs(&conn) {
            eprintln!("❌ 初次同步日誌檔到 SQLite 失敗: {}", e);
        } else {
            println!("✅ SQLite 資料庫已成功載入並完成增量同步！");
        }
    } else {
        eprintln!("❌ 無法連結到 SQLite 資料庫，請檢查 ~/.copilot 是否存在或設定 COPILOT_DIR");
    }

    // 建立 Axum 路由
    let app = Router::new()
        // API 路由
        .route("/api/dates", get(get_available_dates))
        .route("/api/setup-info", get(get_setup_info))
        .route("/api/usage/:date", get(get_usage_details))
        .route("/api/session/:session_id", get(get_session_details))
        .route("/api/months", get(get_available_months))
        .route("/api/monthly/:year_month", get(get_monthly_details))
        .route("/api/sync", get(trigger_manual_sync))
        // 靜態檔案路由： fallback 到 static/index.html，並將所有 / 請求導向 static 目錄
        .nest_service("/static", ServeDir::new("static"))
        .fallback_service(ServeDir::new("static"))
        .layer(CorsLayer::permissive());

    // 監聽本地 3000 Port
    let listener = tokio::net::TcpListener::bind("0.0.0.0:3000").await.unwrap();
    println!("🚀 GitHub Copilot CLI Token Insights Dashboard is running on: http://localhost:3000");
    
    axum::serve(listener, app).await.unwrap();
}

/// 獲取 .copilot 的基準路徑
/// 1. 優先使用環境變數 `COPILOT_DIR`
/// 2. 其次使用主目錄下的 `~/.copilot`
fn get_copilot_dir() -> Result<PathBuf, String> {
    if let Ok(val) = std::env::var("COPILOT_DIR") {
        let p = PathBuf::from(val);
        if p.exists() {
            return Ok(p);
        }
    }

    if let Some(home) = dirs::home_dir() {
        let p = home.join(".copilot");
        if p.exists() {
            return Ok(p);
        }
    }

    // 備用方案：偵測 WSL 下 /home/chenting/.copilot
    let backup = PathBuf::from("/home/chenting/.copilot");
    if backup.exists() {
        return Ok(backup);
    }

    Err("無法定位 .copilot 資料夾，請設定 COPILOT_DIR 環境變數。".to_string())
}

#[derive(Serialize)]
struct SetupInfoResponse {
    workspace_dir: String,
    script_path: String,
    copilot_dir: String,
    copilot_dir_exists: bool,
    home_dir: String,
}

async fn get_setup_info() -> impl IntoResponse {
    let workspace_dir = match std::env::current_dir() {
        Ok(dir) => dir.to_string_lossy().into_owned(),
        Err(_) => "".to_string(),
    };

    let script_path = if !workspace_dir.is_empty() {
        let mut p = PathBuf::from(&workspace_dir);
        p.push("shell");
        p.push("statusline-token.sh");
        p.to_string_lossy().into_owned()
    } else {
        "".to_string()
    };

    let home_dir_path = dirs::home_dir().unwrap_or_else(|| PathBuf::from("/home/user"));
    let home_dir_str = home_dir_path.to_string_lossy().into_owned();

    let copilot_dir_path = home_dir_path.join(".copilot");
    let copilot_dir_exists = copilot_dir_path.exists();
    let copilot_dir_str = copilot_dir_path.to_string_lossy().into_owned();

    Json(SetupInfoResponse {
        workspace_dir,
        script_path,
        copilot_dir: copilot_dir_str,
        copilot_dir_exists,
        home_dir: home_dir_str,
    })
}


/// 解析使用量日誌檔案，相容單行 JSONL 與多行美化（Prettified）JSON 格式
fn parse_usage_entries(content: &str) -> Vec<UsageEntry> {
    let mut entries: Vec<UsageEntry> = Vec::new();
    let mut current_obj = String::new();
    let mut in_object = false;

    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        // 1. 先嘗試單行解析（相容標準 JSONL）
        if !in_object && trimmed.starts_with('{') && trimmed.ends_with('}') {
            match serde_json::from_str::<UsageEntry>(trimmed) {
                Ok(entry) => {
                    entries.push(entry);
                    continue;
                }
                Err(_) => {
                    // 若失敗則嘗試進入多行解析邏輯
                }
            }
        }

        // 2. 多行解析邏輯
        if !in_object {
            if trimmed.starts_with('{') {
                in_object = true;
                current_obj.clear();
                current_obj.push_str(line);
                current_obj.push('\n');
            }
        } else {
            current_obj.push_str(line);
            current_obj.push('\n');

            // 判斷是否為根閉合大括號 `}`（無空格/縮排）
            let is_root_close = line.trim_end() == "}" && !line.starts_with(' ') && !line.starts_with('\t');
            if is_root_close {
                match serde_json::from_str::<UsageEntry>(&current_obj) {
                    Ok(entry) => entries.push(entry),
                    Err(e) => {
                        eprintln!("解析日誌項錯誤 (跳過): {}", e);
                    }
                }
                in_object = false;
                current_obj.clear();
            }
        }
    }
    entries
}

// =========================================================================
// API 1: 列出所有可用的日誌日期
// =========================================================================

#[derive(Serialize)]
struct DateListResponse {
    dates: Vec<String>,
}

async fn get_available_dates() -> impl IntoResponse {
    // 在讀取前做一次增量同步以確保資料最新
    let _ = tokio::task::spawn_blocking(|| {
        if let Ok(conn) = db::get_db_conn() {
            let _ = db::sync_usage_logs(&conn);
        }
    }).await;

    let res: Result<Vec<String>, String> = tokio::task::spawn_blocking(|| {
        let conn = db::get_db_conn()?;
        let mut stmt = conn.prepare("SELECT DISTINCT date FROM usage_entries ORDER BY date DESC")
            .map_err(|e| e.to_string())?;
        
        let dates_iter = stmt.query_map([], |row| row.get::<_, String>(0))
            .map_err(|e| e.to_string())?;
        
        let mut dates = Vec::new();
        for d in dates_iter {
            if let Ok(date) = d {
                dates.push(date);
            }
        }
        Ok(dates)
    }).await.unwrap_or_else(|_| Err("執行緒執行失敗".to_string()));

    match res {
        Ok(dates) => Json(DateListResponse { dates }).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({ "error": e }))).into_response(),
    }
}

// =========================================================================
// API 2: 獲取特定日期的使用量與 Token 狀況
// =========================================================================

#[derive(Serialize, Deserialize, Debug, Clone)]
struct TokenStats {
    input: u64,
    output: u64,
    cache_read: Option<u64>,
    cache_write: Option<u64>,
    reasoning: Option<u64>,
    total: u64,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
struct ContextStats {
    current_context_tokens: Option<u64>,
    displayed_context_limit: Option<u64>,
    current_context_used_percentage: Option<String>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
struct CostStats {
    total_api_duration_ms: Option<f64>,
    total_duration_ms: Option<f64>,
    total_premium_requests: Option<f64>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
struct UsageEntry {
    timestamp: String,
    session_id: String,
    session_name: Option<String>,
    transcript_path: Option<String>,
    cwd: Option<String>,
    version: Option<String>,
    turn_no: u32,
    model: Option<String>,
    model_id: Option<String>,
    tokens: Option<TokenStats>,
    delta_tokens: Option<TokenStats>,
    context: Option<ContextStats>,
    cost: Option<CostStats>,
}

#[derive(Serialize)]
struct UsageDetailsResponse {
    date: String,
    summary: DaySummary,
    sessions: Vec<SessionSummary>,
    raw_entries: Vec<UsageEntry>,
}

#[derive(Serialize, Default)]
struct DaySummary {
    total_sessions: usize,
    total_tokens: u64,
    total_input_tokens: u64,
    total_output_tokens: u64,
    total_reasoning_tokens: u64,
    total_cache_read_tokens: u64,
    total_duration_ms: u64,
    total_requests: u64,
}

#[derive(Serialize)]
struct SessionSummary {
    session_id: String,
    session_name: String,
    cwd: String,
    model: String,
    total_tokens: u64,
    total_input_tokens: u64,
    total_output_tokens: u64,
    total_cache_read_tokens: u64,
    total_reasoning_tokens: u64,
    max_turn_no: u32,
    timestamp: String,
    duration_ms: u64,
}

async fn get_usage_details(Path(date): Path<String>) -> impl IntoResponse {
    // 確保資料最新
    let _ = tokio::task::spawn_blocking(|| {
        if let Ok(conn) = db::get_db_conn() {
            let _ = db::sync_usage_logs(&conn);
        }
    }).await;

    let date_clone = date.clone();
    let entries_res: Result<Vec<UsageEntry>, String> = tokio::task::spawn_blocking(move || {
        let conn = db::get_db_conn()?;
        let mut stmt = conn.prepare(
            "SELECT 
                timestamp, session_id, session_name, transcript_path, cwd, version, turn_no, model, model_id,
                tokens_input, tokens_output, tokens_cache_read, tokens_reasoning, tokens_total,
                delta_input, delta_output, delta_cache_read, delta_reasoning, delta_total,
                duration_ms, premium_requests
             FROM usage_entries WHERE date = ? ORDER BY timestamp ASC"
        ).map_err(|e| e.to_string())?;

        let entries_iter = stmt.query_map(params![date_clone], |row| {
            let tokens_input: Option<u64> = row.get::<_, Option<i64>>(9)?.map(|v| v as u64);
            let tokens_output: Option<u64> = row.get::<_, Option<i64>>(10)?.map(|v| v as u64);
            let tokens_cache_read: Option<u64> = row.get::<_, Option<i64>>(11)?.map(|v| v as u64);
            let tokens_reasoning: Option<u64> = row.get::<_, Option<i64>>(12)?.map(|v| v as u64);
            let tokens_total: Option<u64> = row.get::<_, Option<i64>>(13)?.map(|v| v as u64);

            let tokens = if let (Some(input), Some(output), Some(total)) = (tokens_input, tokens_output, tokens_total) {
                Some(TokenStats {
                    input,
                    output,
                    cache_read: tokens_cache_read,
                    cache_write: None,
                    reasoning: tokens_reasoning,
                    total,
                })
            } else {
                None
            };

            let delta_input: Option<u64> = row.get::<_, Option<i64>>(14)?.map(|v| v as u64);
            let delta_output: Option<u64> = row.get::<_, Option<i64>>(15)?.map(|v| v as u64);
            let delta_cache_read: Option<u64> = row.get::<_, Option<i64>>(16)?.map(|v| v as u64);
            let delta_reasoning: Option<u64> = row.get::<_, Option<i64>>(17)?.map(|v| v as u64);
            let delta_total: Option<u64> = row.get::<_, Option<i64>>(18)?.map(|v| v as u64);

            let delta_tokens = if let (Some(input), Some(output), Some(total)) = (delta_input, delta_output, delta_total) {
                Some(TokenStats {
                    input,
                    output,
                    cache_read: delta_cache_read,
                    cache_write: None,
                    reasoning: delta_reasoning,
                    total,
                })
            } else {
                None
            };

            let duration_ms: Option<f64> = row.get::<_, Option<i64>>(19)?.map(|v| v as f64);
            let premium_requests: Option<f64> = row.get::<_, Option<i64>>(20)?.map(|v| v as f64);

            let cost = if duration_ms.is_some() || premium_requests.is_some() {
                Some(CostStats {
                    total_api_duration_ms: duration_ms,
                    total_duration_ms: None,
                    total_premium_requests: premium_requests,
                })
            } else {
                None
            };

            Ok(UsageEntry {
                timestamp: row.get(0)?,
                session_id: row.get(1)?,
                session_name: row.get(2).ok(),
                transcript_path: row.get(3).ok(),
                cwd: row.get(4).ok(),
                version: row.get(5).ok(),
                turn_no: row.get::<_, i64>(6)? as u32,
                model: row.get(7).ok(),
                model_id: row.get(8).ok(),
                tokens,
                delta_tokens,
                context: None,
                cost,
            })
        }).map_err(|e| e.to_string())?;

        let mut entries = Vec::new();
        for entry in entries_iter {
            if let Ok(e) = entry {
                entries.push(e);
            }
        }
        Ok(entries)
    }).await.unwrap_or_else(|_| Err("執行緒執行失敗".to_string()));

    let entries = match entries_res {
        Ok(e) => e,
        Err(err) => return (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({ "error": err }))).into_response(),
    };

    if entries.is_empty() {
        return (StatusCode::NOT_FOUND, Json(serde_json::json!({ "error": "找不到指定日期的使用日誌。" }))).into_response();
    }

    // 整理當日摘要指標
    let mut summary = DaySummary::default();
    let mut sessions_map: HashMap<String, Vec<UsageEntry>> = HashMap::new();

    for entry in &entries {
        if let Some(ref tokens) = entry.delta_tokens {
            summary.total_tokens += tokens.total;
            summary.total_input_tokens += tokens.input;
            summary.total_output_tokens += tokens.output;
            summary.total_reasoning_tokens += tokens.reasoning.unwrap_or(0);
            summary.total_cache_read_tokens += tokens.cache_read.unwrap_or(0);
        } else if let Some(ref tokens) = entry.tokens {
            if entry.turn_no == 1 {
                summary.total_tokens += tokens.total;
                summary.total_input_tokens += tokens.input;
                summary.total_output_tokens += tokens.output;
                summary.total_reasoning_tokens += tokens.reasoning.unwrap_or(0);
                summary.total_cache_read_tokens += tokens.cache_read.unwrap_or(0);
            }
        }

        let sid = entry.session_id.clone();
        sessions_map.entry(sid).or_default().push(entry.clone());
    }

    summary.total_sessions = sessions_map.len();

    // 整理每個 Session 的統計
    let mut sessions_summary = Vec::new();
    for (session_id, s_entries) in sessions_map {
        let last_entry = s_entries
            .iter()
            .max_by_key(|e| e.turn_no)
            .cloned()
            .unwrap_or_else(|| s_entries[0].clone());

        let session_tokens = s_entries
            .iter()
            .map(|e| e.delta_tokens.as_ref().map(|t| t.total).unwrap_or(0))
            .sum::<u64>();

        let session_input_tokens = s_entries
            .iter()
            .map(|e| e.delta_tokens.as_ref().map(|t| t.input).unwrap_or(0))
            .sum::<u64>();

        let session_output_tokens = s_entries
            .iter()
            .map(|e| e.delta_tokens.as_ref().map(|t| t.output).unwrap_or(0))
            .sum::<u64>();

        let session_cache_read = s_entries
            .iter()
            .map(|e| e.delta_tokens.as_ref().and_then(|t| t.cache_read).unwrap_or(0))
            .sum::<u64>();

        let session_reasoning = s_entries
            .iter()
            .map(|e| e.delta_tokens.as_ref().and_then(|t| t.reasoning).unwrap_or(0))
            .sum::<u64>();

        let session_duration = last_entry
            .cost
            .as_ref()
            .and_then(|c| c.total_api_duration_ms)
            .unwrap_or(0.0) as u64;

        let session_requests = last_entry
            .cost
            .as_ref()
            .and_then(|c| c.total_premium_requests)
            .unwrap_or(0.0) as u64;

        summary.total_duration_ms += session_duration;
        summary.total_requests += session_requests;

        let total_cache_read_tokens = if session_tokens > 0 {
            session_cache_read
        } else {
            last_entry.tokens.as_ref().and_then(|t| t.cache_read).unwrap_or(0)
        };

        let total_reasoning_tokens = if session_tokens > 0 {
            session_reasoning
        } else {
            last_entry.tokens.as_ref().and_then(|t| t.reasoning).unwrap_or(0)
        };

        sessions_summary.push(SessionSummary {
            session_id,
            session_name: last_entry.session_name.unwrap_or_else(|| "Start Coding Session".to_string()),
            cwd: last_entry.cwd.unwrap_or_default(),
            model: last_entry.model.unwrap_or_else(|| "Unknown Model".to_string()),
            total_tokens: if session_tokens > 0 { session_tokens } else { last_entry.tokens.as_ref().map(|t| t.total).unwrap_or(0) },
            total_input_tokens: if session_tokens > 0 { session_input_tokens } else { last_entry.tokens.as_ref().map(|t| t.input).unwrap_or(0) },
            total_output_tokens: if session_tokens > 0 { session_output_tokens } else { last_entry.tokens.as_ref().map(|t| t.output).unwrap_or(0) },
            total_cache_read_tokens,
            total_reasoning_tokens,
            max_turn_no: s_entries.iter().map(|e| e.turn_no).max().unwrap_or(1),
            timestamp: s_entries[0].timestamp.clone(),
            duration_ms: session_duration,
        });
    }

    sessions_summary.sort_by(|a, b| b.timestamp.cmp(&a.timestamp));

    Json(UsageDetailsResponse {
        date,
        summary,
        sessions: sessions_summary,
        raw_entries: entries,
    }).into_response()
}

// =========================================================================
// API 3: 獲取 Session 對話細節並重建 Timeline 時間軸
// =========================================================================

#[derive(Serialize)]
struct SessionTimelineResponse {
    session_id: String,
    metadata: HashMap<String, serde_json::Value>,
    timeline: Vec<TimelineItem>,
}

#[derive(Serialize)]
#[serde(tag = "event_type", content = "event_data")]
enum TimelineItem {
    UserPrompt {
        timestamp: String,
        prompt: String,
        transformed_prompt: Option<String>,
        attachments: Vec<serde_json::Value>,
    },
    AssistantReply {
        timestamp: String,
        reply: String,
        model: String,
        output_tokens: Option<u64>,
        input_tokens: Option<u64>,
        cache_read_tokens: Option<u64>,
        cache_write_tokens: Option<u64>,
        reasoning_tokens: Option<u64>,
        total_tokens: Option<u64>,
        tool_requests: Vec<serde_json::Value>,
    },
    ToolStep {
        timestamp: String,
        tool_name: String,
        arguments: serde_json::Value,
        result: Option<serde_json::Value>,
    },
    SystemStatus {
        timestamp: String,
        status_type: String,
        message: String,
    },
}

async fn get_session_details(Path(session_id): Path<String>) -> impl IntoResponse {
    let copilot_dir = match get_copilot_dir() {
        Ok(dir) => dir,
        Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({ "error": e }))).into_response(),
    };

    let mut filepath = copilot_dir.join("session-state").join(&session_id).join("events.jsonl");
    if !filepath.exists() {
        let fallback = copilot_dir.join("session-state").join(format!("{}.jsonl", session_id));
        if fallback.exists() {
            filepath = fallback;
        } else {
            return (StatusCode::NOT_FOUND, Json(serde_json::json!({ "error": format!("找不到 Session {} 的事件歷史紀錄。", session_id) }))).into_response();
        }
    }

    let file = match File::open(&filepath) {
        Ok(f) => f,
        Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({ "error": format!("開啟檔案失敗: {}", e) }))).into_response(),
    };

    // 預先從 SQLite 資料庫中載入此 Session 的每回合 (turn_no) 的 Token 增量 (delta_tokens) 統計
    let session_id_clone = session_id.clone();
    let db_entries: HashMap<u32, TokenStats> = tokio::task::spawn_blocking(move || {
        let mut map = HashMap::new();
        if let Ok(conn) = db::get_db_conn() {
            if let Ok(mut stmt) = conn.prepare(
                "SELECT turn_no, delta_input, delta_output, delta_cache_read, delta_reasoning, delta_total 
                 FROM usage_entries WHERE session_id = ? ORDER BY turn_no ASC"
            ) {
                if let Ok(mut rows) = stmt.query(params![session_id_clone]) {
                    while let Ok(Some(row)) = rows.next() {
                        if let (Ok(turn_no), Ok(delta_input), Ok(delta_output), Ok(delta_total)) = (
                            row.get::<_, i64>(0),
                            row.get::<_, Option<i64>>(1),
                            row.get::<_, Option<i64>>(2),
                            row.get::<_, Option<i64>>(5)
                        ) {
                            if let (Some(input), Some(output), Some(total)) = (delta_input, delta_output, delta_total) {
                                let cache_read = row.get::<_, Option<i64>>(3).ok().flatten().map(|v| v as u64);
                                let reasoning = row.get::<_, Option<i64>>(4).ok().flatten().map(|v| v as u64);
                                map.insert(turn_no as u32, TokenStats {
                                    input: input as u64,
                                    output: output as u64,
                                    cache_read,
                                    cache_write: None,
                                    reasoning,
                                    total: total as u64,
                                });
                            }
                        }
                    }
                }
            }
        }
        map
    }).await.unwrap_or_default();

    let reader = BufReader::new(file);
    let mut timeline = Vec::new();
    let mut metadata = HashMap::new();

    let mut total_in = 0;
    let mut total_out = 0;
    let mut total_cache = 0;
    let mut total_reasoning = 0;
    let mut total_all = 0;

    // 用於關聯 ToolStep 的狀態對應表 (toolCallId -> TimelineItem 索引)
    let mut tool_calls_map: HashMap<String, usize> = HashMap::new();
    
    // 用於記錄目前對話的回合序號（由 user.message 觸發遞增），以確保與 SQLite 中的 turn_no 完美精確對齊
    let mut current_turn_no = 0;

    for line_res in reader.lines() {
        let line = match line_res {
            Ok(l) => l,
            Err(_) => continue,
        };

        let event: serde_json::Value = match serde_json::from_str(&line) {
            Ok(v) => v,
            Err(_) => continue,
        };

        let event_type = event.get("type").and_then(|t| t.as_str()).unwrap_or("");
        let timestamp = event.get("timestamp").and_then(|t| t.as_str()).unwrap_or("").to_string();
        let data = event.get("data");

        match event_type {
            "session.start" => {
                if let Some(d) = data {
                    metadata.insert("start_time".to_string(), d.get("startTime").cloned().unwrap_or_default());
                    metadata.insert("copilot_version".to_string(), d.get("copilotVersion").cloned().unwrap_or_default());
                    metadata.insert("selected_model".to_string(), d.get("selectedModel").cloned().unwrap_or_default());
                    if let Some(ctx) = d.get("context") {
                        metadata.insert("cwd".to_string(), ctx.get("cwd").cloned().unwrap_or_default());
                        metadata.insert("git_branch".to_string(), ctx.get("branch").cloned().unwrap_or_default());
                        metadata.insert("repository".to_string(), ctx.get("repository").cloned().unwrap_or_default());
                    }
                }
                timeline.push(TimelineItem::SystemStatus {
                    timestamp,
                    status_type: "session_start".to_string(),
                    message: "會話開始 (Session Started)".to_string(),
                });
            }
            "session.shutdown" => {
                timeline.push(TimelineItem::SystemStatus {
                    timestamp,
                    status_type: "session_shutdown".to_string(),
                    message: "會話結束 (Session Ended)".to_string(),
                });
            }
            "user.message" => {
                current_turn_no += 1;
                if let Some(d) = data {
                    let prompt = d.get("content").and_then(|c| c.as_str()).unwrap_or("").to_string();
                    let transformed_prompt = d.get("transformedContent").and_then(|c| c.as_str()).map(|s| s.to_string());
                    let attachments = d.get("attachments").and_then(|a| a.as_array()).cloned().unwrap_or_default();

                    timeline.push(TimelineItem::UserPrompt {
                        timestamp,
                        prompt,
                        transformed_prompt,
                        attachments,
                    });
                }
            }
            "assistant.message" => {
                let active_turn = if current_turn_no == 0 { 1 } else { current_turn_no };
                if let Some(d) = data {
                    let reply = d.get("content").and_then(|c| c.as_str()).unwrap_or("").to_string();
                    let model = d.get("model").and_then(|m| m.as_str()).unwrap_or("GPT").to_string();

                    // 支援扁平屬性與巢狀屬性解析 Token 數據
                    let mut output_tokens = d.get("outputTokens").and_then(|o| o.as_u64());
                    let mut input_tokens = d.get("inputTokens").and_then(|o| o.as_u64());
                    let mut cache_read_tokens = d.get("cacheReadTokens").and_then(|o| o.as_u64());
                    let mut cache_write_tokens = d.get("cacheWriteTokens").and_then(|o| o.as_u64());
                    let mut reasoning_tokens = d.get("reasoningTokens").and_then(|o| o.as_u64());
                    let mut total_tokens = d.get("totalTokens").and_then(|o| o.as_u64());

                    if let Some(tokens_obj) = d.get("tokens") {
                        if output_tokens.is_none() {
                            output_tokens = tokens_obj.get("output").and_then(|t| t.as_u64());
                        }
                        if input_tokens.is_none() {
                            input_tokens = tokens_obj.get("input").and_then(|t| t.as_u64());
                        }
                        if cache_read_tokens.is_none() {
                            cache_read_tokens = tokens_obj.get("cache_read").and_then(|t| t.as_u64());
                        }
                        if cache_write_tokens.is_none() {
                            cache_write_tokens = tokens_obj.get("cache_write").and_then(|t| t.as_u64());
                        }
                        if reasoning_tokens.is_none() {
                            reasoning_tokens = tokens_obj.get("reasoning").and_then(|t| t.as_u64());
                        }
                        if total_tokens.is_none() {
                            total_tokens = tokens_obj.get("total").and_then(|t| t.as_u64());
                        }
                    }

                    // 如果從 events.jsonl 本身解析出的 Token 數據不齊全，則嘗試從 SQLite 資料庫中對應 turn_no 的 delta_tokens 數據補齊
                    if let Some(db_stats) = db_entries.get(&active_turn) {
                        if input_tokens.is_none() || input_tokens == Some(0) {
                            input_tokens = Some(db_stats.input);
                        }
                        if output_tokens.is_none() || output_tokens == Some(0) {
                            output_tokens = Some(db_stats.output);
                        }
                        if cache_read_tokens.is_none() || cache_read_tokens == Some(0) {
                            cache_read_tokens = db_stats.cache_read;
                        }
                        if reasoning_tokens.is_none() || reasoning_tokens == Some(0) {
                            reasoning_tokens = db_stats.reasoning;
                        }
                        if total_tokens.is_none() || total_tokens == Some(0) {
                            total_tokens = Some(db_stats.total);
                        }
                    }

                    if total_tokens.is_none() {
                        if let (Some(in_t), Some(out_t)) = (input_tokens, output_tokens) {
                            total_tokens = Some(in_t + out_t);
                        }
                    }

                    if let Some(t) = input_tokens {
                        total_in += t;
                    }
                    if let Some(t) = output_tokens {
                        total_out += t;
                    }
                    if let Some(t) = cache_read_tokens {
                        total_cache += t;
                    }
                    if let Some(t) = reasoning_tokens {
                        total_reasoning += t;
                    }
                    if let Some(t) = total_tokens {
                        total_all += t;
                    }

                    let tool_requests = d.get("toolRequests").and_then(|r| r.as_array()).cloned().unwrap_or_default();

                    // 即使助理回覆是空白（例如僅呼叫 Tool），也記錄下來方便觀測
                    timeline.push(TimelineItem::AssistantReply {
                        timestamp,
                        reply,
                        model,
                        output_tokens,
                        input_tokens,
                        cache_read_tokens,
                        cache_write_tokens,
                        reasoning_tokens,
                        total_tokens,
                        tool_requests,
                    });
                }
            }
            "tool.execution_start" => {
                if let Some(d) = data {
                    let tool_name = d.get("toolName").and_then(|t| t.as_str()).unwrap_or("unknown").to_string();
                    let arguments = d.get("arguments").cloned().unwrap_or(serde_json::Value::Null);
                    let tool_call_id = d.get("toolCallId").and_then(|id| id.as_str()).unwrap_or("").to_string();

                    let index = timeline.len();
                    timeline.push(TimelineItem::ToolStep {
                        timestamp,
                        tool_name,
                        arguments,
                        result: None,
                    });

                    if !tool_call_id.is_empty() {
                        tool_calls_map.insert(tool_call_id, index);
                    }
                }
            }
            "tool.execution_complete" => {
                if let Some(d) = data {
                    let tool_call_id = d.get("toolCallId").and_then(|id| id.as_str()).unwrap_or("").to_string();
                    let result = d.get("result").cloned().unwrap_or(serde_json::Value::Null);

                    if let Some(&idx) = tool_calls_map.get(&tool_call_id) {
                        if idx < timeline.len() {
                            // 更新先前加入的 ToolStep 結果
                            if let TimelineItem::ToolStep { result: ref mut res, .. } = &mut timeline[idx] {
                                *res = Some(result);
                            } else {
                                // 以防型別不對，直接替換
                                if let TimelineItem::ToolStep { timestamp, tool_name, arguments, .. } = &timeline[idx] {
                                    timeline[idx] = TimelineItem::ToolStep {
                                        timestamp: timestamp.clone(),
                                        tool_name: tool_name.clone(),
                                        arguments: arguments.clone(),
                                        result: Some(result),
                                    };
                                }
                            }
                        }
                    }
                }
            }
            _ => {}
        }
    }

    metadata.insert("total_input_tokens".to_string(), serde_json::Value::from(total_in));
    metadata.insert("total_output_tokens".to_string(), serde_json::Value::from(total_out));
    metadata.insert("total_cache_read_tokens".to_string(), serde_json::Value::from(total_cache));
    metadata.insert("total_reasoning_tokens".to_string(), serde_json::Value::from(total_reasoning));
    metadata.insert("total_tokens".to_string(), serde_json::Value::from(total_all));

    Json(SessionTimelineResponse {
        session_id,
        metadata,
        timeline,
    }).into_response()
}

// =========================================================================
// API 4: 列出所有可用的月份清單
// =========================================================================

#[derive(Serialize)]
struct MonthListResponse {
    months: Vec<String>,
}

async fn get_available_months() -> impl IntoResponse {
    // 確保資料最新
    let _ = tokio::task::spawn_blocking(|| {
        if let Ok(conn) = db::get_db_conn() {
            let _ = db::sync_usage_logs(&conn);
        }
    }).await;

    let res: Result<Vec<String>, String> = tokio::task::spawn_blocking(|| {
        let conn = db::get_db_conn()?;
        let mut stmt = conn.prepare("SELECT DISTINCT substr(date, 1, 7) AS month FROM usage_entries ORDER BY month DESC")
            .map_err(|e| e.to_string())?;
        
        let months_iter = stmt.query_map([], |row| row.get::<_, String>(0))
            .map_err(|e| e.to_string())?;
        
        let mut months = Vec::new();
        for m in months_iter {
            if let Ok(month) = m {
                months.push(month);
            }
        }
        Ok(months)
    }).await.unwrap_or_else(|_| Err("執行緒執行失敗".to_string()));

    match res {
        Ok(month_list) => Json(MonthListResponse { months: month_list }).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({ "error": e }))).into_response(),
    }
}

// =========================================================================
// API 5: 獲取特定月份的彙整統計
// =========================================================================

#[derive(Serialize)]
struct MonthlyDetailsResponse {
    year_month: String,
    summary: DaySummary,
    daily_breakdown: Vec<DailyBreakdownEntry>,
    top_models: Vec<ModelUsageSummary>,
    top_projects: Vec<ProjectUsageSummary>,
}

#[derive(Serialize, Clone)]
struct DailyBreakdownEntry {
    date: String,
    total_sessions: usize,
    total_tokens: u64,
    total_input_tokens: u64,
    total_output_tokens: u64,
    total_cache_read_tokens: u64,
    total_reasoning_tokens: u64,
    total_duration_ms: u64,
    total_requests: u64,
}

#[derive(Serialize, Clone)]
struct ModelUsageSummary {
    model: String,
    session_count: usize,
    total_tokens: u64,
    total_cache_read_tokens: u64,
}

#[derive(Serialize, Clone)]
struct ProjectUsageSummary {
    project: String,
    session_count: usize,
    total_tokens: u64,
    total_cache_read_tokens: u64,
}

async fn get_monthly_details(Path(year_month): Path<String>) -> impl IntoResponse {
    // 確保資料最新
    let _ = tokio::task::spawn_blocking(|| {
        if let Ok(conn) = db::get_db_conn() {
            let _ = db::sync_usage_logs(&conn);
        }
    }).await;

    let query_month = format!("{}-%", year_month);
    let entries_res: Result<Vec<UsageEntry>, String> = tokio::task::spawn_blocking(move || {
        let conn = db::get_db_conn()?;
        let mut stmt = conn.prepare(
            "SELECT 
                timestamp, session_id, session_name, transcript_path, cwd, version, turn_no, model, model_id,
                tokens_input, tokens_output, tokens_cache_read, tokens_reasoning, tokens_total,
                delta_input, delta_output, delta_cache_read, delta_reasoning, delta_total,
                duration_ms, premium_requests
             FROM usage_entries WHERE date LIKE ? ORDER BY timestamp ASC"
        ).map_err(|e| e.to_string())?;

        let entries_iter = stmt.query_map(params![query_month], |row| {
            let tokens_input: Option<u64> = row.get::<_, Option<i64>>(9)?.map(|v| v as u64);
            let tokens_output: Option<u64> = row.get::<_, Option<i64>>(10)?.map(|v| v as u64);
            let tokens_cache_read: Option<u64> = row.get::<_, Option<i64>>(11)?.map(|v| v as u64);
            let tokens_reasoning: Option<u64> = row.get::<_, Option<i64>>(12)?.map(|v| v as u64);
            let tokens_total: Option<u64> = row.get::<_, Option<i64>>(13)?.map(|v| v as u64);

            let tokens = if let (Some(input), Some(output), Some(total)) = (tokens_input, tokens_output, tokens_total) {
                Some(TokenStats {
                    input,
                    output,
                    cache_read: tokens_cache_read,
                    cache_write: None,
                    reasoning: tokens_reasoning,
                    total,
                })
            } else {
                None
            };

            let delta_input: Option<u64> = row.get::<_, Option<i64>>(14)?.map(|v| v as u64);
            let delta_output: Option<u64> = row.get::<_, Option<i64>>(15)?.map(|v| v as u64);
            let delta_cache_read: Option<u64> = row.get::<_, Option<i64>>(16)?.map(|v| v as u64);
            let delta_reasoning: Option<u64> = row.get::<_, Option<i64>>(17)?.map(|v| v as u64);
            let delta_total: Option<u64> = row.get::<_, Option<i64>>(18)?.map(|v| v as u64);

            let delta_tokens = if let (Some(input), Some(output), Some(total)) = (delta_input, delta_output, delta_total) {
                Some(TokenStats {
                    input,
                    output,
                    cache_read: delta_cache_read,
                    cache_write: None,
                    reasoning: delta_reasoning,
                    total,
                })
            } else {
                None
            };

            let duration_ms: Option<f64> = row.get::<_, Option<i64>>(19)?.map(|v| v as f64);
            let premium_requests: Option<f64> = row.get::<_, Option<i64>>(20)?.map(|v| v as f64);

            let cost = if duration_ms.is_some() || premium_requests.is_some() {
                Some(CostStats {
                    total_api_duration_ms: duration_ms,
                    total_duration_ms: None,
                    total_premium_requests: premium_requests,
                })
            } else {
                None
            };

            Ok(UsageEntry {
                timestamp: row.get(0)?,
                session_id: row.get(1)?,
                session_name: row.get(2).ok(),
                transcript_path: row.get(3).ok(),
                cwd: row.get(4).ok(),
                version: row.get(5).ok(),
                turn_no: row.get::<_, i64>(6)? as u32,
                model: row.get(7).ok(),
                model_id: row.get(8).ok(),
                tokens,
                delta_tokens,
                context: None,
                cost,
            })
        }).map_err(|e| e.to_string())?;

        let mut entries = Vec::new();
        for entry in entries_iter {
            if let Ok(e) = entry {
                entries.push(e);
            }
        }
        Ok(entries)
    }).await.unwrap_or_else(|_| Err("執行緒執行失敗".to_string()));

    let entries = match entries_res {
        Ok(e) => e,
        Err(err) => return (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({ "error": err }))).into_response(),
    };

    if entries.is_empty() {
        return (StatusCode::NOT_FOUND, Json(serde_json::json!({ "error": "找不到該月份的使用量資料。" }))).into_response();
    }

    let mut daily_map: HashMap<String, Vec<UsageEntry>> = HashMap::new();
    for e in &entries {
        if e.timestamp.len() >= 10 {
            let d = e.timestamp[0..10].to_string();
            daily_map.entry(d).or_default().push(e.clone());
        }
    }

    let mut daily_breakdown = Vec::new();
    let mut monthly_summary = DaySummary::default();
    
    let mut model_sessions: HashMap<String, std::collections::HashSet<String>> = HashMap::new();
    let mut model_tokens: HashMap<String, u64> = HashMap::new();
    let mut model_cache_tokens: HashMap<String, u64> = HashMap::new();
    
    let mut project_sessions: HashMap<String, std::collections::HashSet<String>> = HashMap::new();
    let mut project_tokens: HashMap<String, u64> = HashMap::new();
    let mut project_cache_tokens: HashMap<String, u64> = HashMap::new();

    // 依日期小到大排序
    let mut sorted_dates: Vec<String> = daily_map.keys().cloned().collect();
    sorted_dates.sort();

    for date_str in sorted_dates {
        let entries_list = daily_map.get(&date_str).unwrap();

        let mut day_tokens = 0;
        let mut day_input = 0;
        let mut day_output = 0;
        let mut day_reasoning = 0;
        let mut day_cache_read = 0;
        let mut day_duration = 0;
        let mut day_requests = 0;
        let mut day_sessions = std::collections::HashSet::new();

        for e in entries_list {
            let sid = e.session_id.clone();
            day_sessions.insert(sid.clone());

            let mut entry_tokens = 0;
            if let Some(ref tokens) = e.delta_tokens {
                entry_tokens = tokens.total;
                day_tokens += tokens.total;
                day_input += tokens.input;
                day_output += tokens.output;
                day_reasoning += tokens.reasoning.unwrap_or(0);
                day_cache_read += tokens.cache_read.unwrap_or(0);
            } else if let Some(ref tokens) = e.tokens {
                if e.turn_no == 1 {
                    entry_tokens = tokens.total;
                    day_tokens += tokens.total;
                    day_input += tokens.input;
                    day_output += tokens.output;
                    day_reasoning += tokens.reasoning.unwrap_or(0);
                    day_cache_read += tokens.cache_read.unwrap_or(0);
                }
            }

            let mut entry_cache = 0;
            if let Some(ref tokens) = e.delta_tokens {
                entry_cache = tokens.cache_read.unwrap_or(0);
            } else if let Some(ref tokens) = e.tokens {
                if e.turn_no == 1 {
                    entry_cache = tokens.cache_read.unwrap_or(0);
                }
            }

            let model = e.model.clone().unwrap_or_else(|| "Unknown Model".to_string());
            model_sessions.entry(model.clone()).or_default().insert(sid.clone());
            *model_tokens.entry(model.clone()).or_default() += entry_tokens;
            *model_cache_tokens.entry(model).or_default() += entry_cache;

            let cwd = e.cwd.clone().unwrap_or_else(|| "Unknown Path".to_string());
            project_sessions.entry(cwd.clone()).or_default().insert(sid.clone());
            *project_tokens.entry(cwd.clone()).or_default() += entry_tokens;
            *project_cache_tokens.entry(cwd).or_default() += entry_cache;
        }

        let mut session_last_entries: std::collections::HashMap<String, UsageEntry> = std::collections::HashMap::new();
        for e in entries_list {
            let sid = e.session_id.clone();
            let entry = session_last_entries.entry(sid).or_insert_with(|| e.clone());
            if e.turn_no > entry.turn_no {
                *entry = e.clone();
            }
        }
        for (_, last_entry) in session_last_entries {
            if let Some(ref cost) = last_entry.cost {
                day_duration += cost.total_api_duration_ms.unwrap_or(0.0) as u64;
                day_requests += cost.total_premium_requests.unwrap_or(0.0) as u64;
            }
        }

        monthly_summary.total_tokens += day_tokens;
        monthly_summary.total_input_tokens += day_input;
        monthly_summary.total_output_tokens += day_output;
        monthly_summary.total_reasoning_tokens += day_reasoning;
        monthly_summary.total_cache_read_tokens += day_cache_read;
        monthly_summary.total_duration_ms += day_duration;
        monthly_summary.total_requests += day_requests;

        daily_breakdown.push(DailyBreakdownEntry {
            date: date_str,
            total_sessions: day_sessions.len(),
            total_tokens: day_tokens,
            total_input_tokens: day_input,
            total_output_tokens: day_output,
            total_cache_read_tokens: day_cache_read,
            total_reasoning_tokens: day_reasoning,
            total_duration_ms: day_duration,
            total_requests: day_requests,
        });
    }

    let mut all_month_sessions = std::collections::HashSet::new();
    for (_, sids) in &model_sessions {
        for sid in sids {
            all_month_sessions.insert(sid.clone());
        }
    }
    monthly_summary.total_sessions = all_month_sessions.len();

    let mut top_models = Vec::new();
    for (model, sids) in model_sessions {
        let total_tokens = model_tokens.get(&model).cloned().unwrap_or(0);
        let total_cache_read_tokens = model_cache_tokens.get(&model).cloned().unwrap_or(0);
        top_models.push(ModelUsageSummary {
            model,
            session_count: sids.len(),
            total_tokens,
            total_cache_read_tokens,
        });
    }
    top_models.sort_by(|a, b| b.total_tokens.cmp(&a.total_tokens));

    let mut top_projects = Vec::new();
    for (project, sids) in project_sessions {
        let total_tokens = project_tokens.get(&project).cloned().unwrap_or(0);
        let total_cache_read_tokens = project_cache_tokens.get(&project).cloned().unwrap_or(0);
        top_projects.push(ProjectUsageSummary {
            project,
            session_count: sids.len(),
            total_tokens,
            total_cache_read_tokens,
        });
    }
    top_projects.sort_by(|a, b| b.total_tokens.cmp(&a.total_tokens));

    Json(MonthlyDetailsResponse {
        year_month,
        summary: monthly_summary,
        daily_breakdown,
        top_models,
        top_projects,
    }).into_response()
}

async fn trigger_manual_sync() -> impl IntoResponse {
    let res = tokio::task::spawn_blocking(|| {
        let conn = db::get_db_conn()?;
        db::init_db(&conn)?; // 確保資料庫與資料表已被成功初始化（若檔案被刪除會自動重建）
        db::sync_usage_logs(&conn)
    }).await.unwrap_or_else(|_| Err("執行緒執行失敗".to_string()));

    match res {
        Ok(_) => (StatusCode::OK, Json(serde_json::json!({ "status": "success", "message": "資料庫增量同步已完成！" }))).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({ "status": "error", "error": e }))).into_response(),
    }
}
