#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use tauri::{
    menu::{Menu, MenuBuilder, MenuItem, MenuItemBuilder},
    tray::TrayIconBuilder,
    AppHandle, Emitter, Manager, State,
};
use tokio::sync::Mutex;

static TEST_RUNNING: AtomicBool = AtomicBool::new(false);

// 持有菜单栏结果项，测试完成后更新文本
struct TrayState {
    result_items: std::sync::Mutex<Vec<MenuItem<tauri::Wry>>>,
}


#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelConfig {
    pub id: String,
    pub name: String,
    pub api_type: String, // "openai" or "anthropic"
    pub base_url: String,
    pub api_key: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Settings {
    pub test_interval_minutes: u32,
    pub test_duration_seconds: u32,
    pub notify_on_complete: bool,
    pub fixed_prompt: String,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            test_interval_minutes: 30,
            test_duration_seconds: 60,
            notify_on_complete: false,
            fixed_prompt: "Say 'Token speed test successful' and count from 1 to 100.".to_string(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TestResult {
    pub model_id: String,
    pub model_name: String,
    pub tok_per_sec: f64,
    pub total_time_ms: u64,
    pub ttft_ms: u64,
    pub timestamp: DateTime<Utc>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HistoryRecord {
    pub results: Vec<TestResult>,
    pub tested_at: DateTime<Utc>,
}

#[derive(Debug, Serialize)]
pub struct AppError {
    pub message: String,
}

fn get_data_dir() -> PathBuf {
    let data_dir = dirs::data_local_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("model-speed");
    fs::create_dir_all(&data_dir).ok();
    data_dir
}

fn get_config_path() -> PathBuf {
    get_data_dir().join("config.json")
}

fn get_settings_path() -> PathBuf {
    get_data_dir().join("settings.json")
}

fn get_history_path() -> PathBuf {
    get_data_dir().join("history.json")
}

fn load_models() -> Vec<ModelConfig> {
    let path = get_config_path();
    if path.exists() {
        fs::read_to_string(&path)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    } else {
        vec![]
    }
}

fn save_models(models: &[ModelConfig]) -> Result<(), String> {
    let path = get_config_path();
    let json = serde_json::to_string_pretty(models).map_err(|e| e.to_string())?;
    fs::write(path, json).map_err(|e| e.to_string())?;
    Ok(())
}

fn load_settings() -> Settings {
    let path = get_settings_path();
    if path.exists() {
        fs::read_to_string(&path)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    } else {
        Settings::default()
    }
}

fn save_settings(settings: &Settings) -> Result<(), String> {
    let path = get_settings_path();
    let json = serde_json::to_string_pretty(settings).map_err(|e| e.to_string())?;
    fs::write(path, json).map_err(|e| e.to_string())?;
    Ok(())
}

fn load_history() -> Vec<HistoryRecord> {
    let path = get_history_path();
    if path.exists() {
        fs::read_to_string(&path)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    } else {
        vec![]
    }
}

fn save_history(history: &[HistoryRecord]) -> Result<(), String> {
    let path = get_history_path();
    let json = serde_json::to_string_pretty(history).map_err(|e| e.to_string())?;
    fs::write(path, json).map_err(|e| e.to_string())?;
    Ok(())
}

async fn test_model(model: &ModelConfig, prompt: &str) -> TestResult {
    eprintln!("[test_model] start: {} ({})", model.name, model.api_type);
    let start = std::time::Instant::now();
    let mut ttft_ms: u64 = 0;
    let mut total_tokens: u64 = 0;
    let mut error_msg: Option<String> = None;

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(120))
        .build()
        .unwrap_or_default();

    match model.api_type.as_str() {
        "openai" => {
            let url = format!("{}/chat/completions", model.base_url.trim_end_matches('/'));
            let request_body = serde_json::json!({
                "model": model.id,
                "messages": [{"role": "user", "content": prompt}],
                "stream": true,
                "stream_options": {"include_usage": true},
                "max_tokens": 1000
            });

            let response = client
                .post(&url)
                .header("Authorization", format!("Bearer {}", model.api_key))
                .header("Content-Type", "application/json")
                .json(&request_body)
                .send()
                .await;

            match response {
                Ok(resp) => {
                    if !resp.status().is_success() {
                        let status = resp.status();
                        let body = resp.text().await.unwrap_or_default();
                        error_msg = Some(format!("HTTP {} : {}", status, body));
                    } else {
                        let mut stream = resp.bytes_stream();
                        let mut buf = String::new();
                        let mut done = false;
                        while !done {
                            let chunk = stream.next().await;
                            match chunk {
                                None => { done = true; }
                                Some(Err(e)) => { error_msg = Some(e.to_string()); done = true; }
                                Some(Ok(bytes)) => {
                                    if let Ok(text) = String::from_utf8(bytes.to_vec()) {
                                        buf.push_str(&text);
                                        let mut newline_idx;
                                        loop {
                                            newline_idx = buf.find('\n');
                                            if newline_idx.is_none() { break; }
                                            let line = buf[..newline_idx.unwrap()].trim().to_string();
                                            buf.drain(..=newline_idx.unwrap());
                                            if line.is_empty() { continue; }
                                            if !line.starts_with("data: ") { continue; }
                                            let data = &line[6..];
                                            if data == "[DONE]" { done = true; break; }
                                            if ttft_ms == 0 { ttft_ms = start.elapsed().as_millis() as u64; }
                                            if let Ok(event) = serde_json::from_str::<serde_json::Value>(data) {
                                                if let Some(s) = event.get("choices")
                                                    .and_then(|c| c.get(0))
                                                    .and_then(|c| c.get("delta"))
                                                    .and_then(|d| d.get("content"))
                                                    .and_then(|c| c.as_str())
                                                {
                                                    total_tokens += s.chars().count() as u64 / 4;
                                                }
                                                if let Some(n) = event.get("usage")
                                                    .and_then(|u| u.get("completion_tokens"))
                                                    .and_then(|c| c.as_u64())
                                                {
                                                    total_tokens = n;
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
                Err(e) => { error_msg = Some(e.to_string()); }
            }
        }
        "anthropic" => {
            let url = format!("{}/v1/messages", model.base_url.trim_end_matches('/'));
            let request_body = serde_json::json!({
                "model": model.id,
                "messages": [{"role": "user", "content": prompt}],
                "stream": true,
                "max_tokens": 1024
            });

            let response = client
                .post(&url)
                .header("x-api-key", &model.api_key)
                .header("anthropic-version", "2023-06-01")
                .header("Content-Type", "application/json")
                .json(&request_body)
                .send()
                .await;

            match response {
                Ok(resp) => {
                    if !resp.status().is_success() {
                        let status = resp.status();
                        let body = resp.text().await.unwrap_or_default();
                        error_msg = Some(format!("HTTP {} : {}", status, body));
                    } else {
                        let mut stream = resp.bytes_stream();
                        let mut buf = String::new();
                        let mut done = false;
                        while !done {
                            let chunk = stream.next().await;
                            match chunk {
                                None => { done = true; }
                                Some(Err(e)) => { error_msg = Some(e.to_string()); done = true; }
                                Some(Ok(bytes)) => {
                                    if let Ok(text) = String::from_utf8(bytes.to_vec()) {
                                        buf.push_str(&text);
                                        let mut newline_idx;
                                        loop {
                                            newline_idx = buf.find('\n');
                                            if newline_idx.is_none() { break; }
                                            let line = buf[..newline_idx.unwrap()].trim().to_string();
                                            buf.drain(..=newline_idx.unwrap());
                                            if line.is_empty() { continue; }
                                            if !line.starts_with("data: ") { continue; }
                                            let data = &line[6..];
                                            if data == "[DONE]" { done = true; break; }
                                            if ttft_ms == 0 { ttft_ms = start.elapsed().as_millis() as u64; }
                                            if let Ok(event) = serde_json::from_str::<serde_json::Value>(data) {
                                                let evt_type = event.get("type").and_then(|t| t.as_str()).unwrap_or("");
                                                if evt_type == "message_start" {
                                                    if let Some(n) = event.pointer("/message/usage/input_tokens").and_then(|v| v.as_u64()) {
                                                        let _ = n; // input tokens, ignore for output speed
                                                    }
                                                }
                                                if evt_type == "message_delta" {
                                                    if let Some(n) = event.pointer("/usage/output_tokens").and_then(|v| v.as_u64()) {
                                                        total_tokens = n;
                                                    }
                                                }
                                                if evt_type == "content_block_delta" {
                                                    if let Some(s) = event.get("delta").and_then(|d| d.get("text")).and_then(|t| t.as_str()) {
                                                        if total_tokens == 0 {
                                                            total_tokens += s.chars().count() as u64 / 4;
                                                        }
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
                Err(e) => { error_msg = Some(e.to_string()); }
            }
        }
        _ => { error_msg = Some("Unknown API type".to_string()); }
    }

    let total_time_ms = start.elapsed().as_millis() as u64;
    let tok_per_sec = if total_tokens > 0 && total_time_ms > 0 {
        (total_tokens as f64 / total_time_ms as f64) * 1000.0
    } else {
        0.0
    };

    eprintln!("[test_model] done: {} tokens={} ttft={}ms total={}ms err={:?}", model.name, total_tokens, ttft_ms, total_time_ms, error_msg);
    TestResult {
        model_id: model.id.clone(),
        model_name: model.name.clone(),
        tok_per_sec,
        total_time_ms,
        ttft_ms,
        timestamp: Utc::now(),
        error: error_msg,
    }
}

use futures_util::StreamExt;

struct AppState {
    models: Mutex<Vec<ModelConfig>>,
    settings: Mutex<Settings>,
}

#[tauri::command]
async fn get_models(state: State<'_, AppState>) -> Result<Vec<ModelConfig>, String> {
    let models = state.models.lock().await;
    Ok(models.clone())
}

#[tauri::command]
async fn save_models_cmd(
    app: AppHandle,
    state: State<'_, AppState>,
    models: Vec<ModelConfig>,
) -> Result<(), String> {
    {
        let mut current = state.models.lock().await;
        *current = models.clone();
    }
    save_models(&models)?;
    // 模型列表变更后重建托盘菜单，确保所有模型都显示
    rebuild_tray_menu(&app).map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
async fn get_settings(state: State<'_, AppState>) -> Result<Settings, String> {
    let settings = state.settings.lock().await;
    Ok(settings.clone())
}

#[tauri::command]
async fn save_settings_cmd(
    state: State<'_, AppState>,
    settings: Settings,
) -> Result<(), String> {
    let mut current = state.settings.lock().await;
    *current = settings.clone();
    save_settings(&settings)
}

#[tauri::command]
async fn get_history() -> Result<Vec<HistoryRecord>, String> {
    Ok(load_history())
}

#[tauri::command]
async fn export_history() -> Result<String, String> {
    let history = load_history();
    serde_json::to_string_pretty(&history).map_err(|e| e.to_string())
}

#[tauri::command]
async fn clear_history() -> Result<(), String> {
    save_history(&[])
}

#[tauri::command]
async fn run_test(app: AppHandle) -> Result<String, String> {
    if TEST_RUNNING.load(Ordering::SeqCst) {
        return Err("Test already running".to_string());
    }

    TEST_RUNNING.store(true, Ordering::SeqCst);

    let state = app.state::<AppState>();
    let models = {
        let m = state.models.lock().await;
        m.clone()
    };

    let settings = {
        let s = state.settings.lock().await;
        s.clone()
    };

    if models.is_empty() {
        TEST_RUNNING.store(false, Ordering::SeqCst);
        return Err("No models configured".to_string());
    }

    let app_clone = app.clone();

    tokio::spawn(async move {
        // 并发测试所有模型
        let mut futs = Vec::new();
        for model in models.iter() {
            let m = model.clone();
            let prompt = settings.fixed_prompt.clone();
            futs.push(async move {
                let result = test_model(&m, &prompt).await;
                (m.name.clone(), result)
            });
        }
        eprintln!("[run_test] waiting for {} models", futs.len());
        let results_raw = futures_util::future::join_all(futs).await;
        eprintln!("[run_test] all models done");

        // 按完成顺序实时推送，最终按原始顺序收集
        let mut results = Vec::new();
        for (_, result) in results_raw {
            let _ = app_clone.emit("test-progress", &result);
            results.push(result);
        }

        // 更新菜单栏显示最新结果（在 results 被 move 之前）
        update_tray_results(&app, &results);

        let record = HistoryRecord {
            results,
            tested_at: Utc::now(),
        };

        let mut history = load_history();
        history.push(record);
        if history.len() > 1000 {
            history = history.into_iter().rev().take(1000).rev().collect();
        }
        let _ = save_history(&history);

        if settings.notify_on_complete {
            let _ = app.emit("test-complete", ());
        }

        TEST_RUNNING.store(false, Ordering::SeqCst);
        let _ = app.emit("test-finished", ());
    });

    Ok("Test started".to_string())
}

#[tauri::command]
fn is_test_running() -> bool {
    TEST_RUNNING.load(Ordering::SeqCst)
}

fn update_tray_results(app: &AppHandle, results: &[TestResult]) {
    let tray_state = app.state::<TrayState>();
    let items = tray_state.result_items.lock().unwrap();
    // 按 menu item 顺序匹配结果（顺序与配置的模型一致）
    for (i, item) in items.iter().enumerate() {
        let text = match results.get(i) {
            Some(r) => {
                if r.error.is_some() {
                    format!("{}: ERR", truncate(&r.model_name, 20))
                } else {
                    format!("{}: {:.1} tok/s", truncate(&r.model_name, 20), r.tok_per_sec)
                }
            }
            None => "—".to_string(),
        };
        let _ = item.set_text(text);
    }
}

/// 托盘菜单事件处理：quit / show / test
fn handle_tray_menu_event(app: &AppHandle, event: tauri::menu::MenuEvent) {
    match event.id.as_ref() {
        "quit" => {
            app.exit(0);
        }
        "show" => {
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.show();
                let _ = window.set_focus();
            }
        }
        "test" => {
            let app_clone = app.clone();
            tauri::async_runtime::spawn(async move {
                let _ = run_test(app_clone).await;
            });
        }
        _ => {}
    }
}

/// 构建托盘菜单：Show Window / Run Test Now / 每个模型一项 / Quit。
/// 返回菜单和结果项（结果项存入 state，供 update_tray_results 按 index 更新文本）。
/// enabled=true 让结果项文字以正常（白色）显示，disabled 会被系统渲染成灰色。
fn build_tray_menu(app: &AppHandle) -> tauri::Result<(Menu<tauri::Wry>, Vec<MenuItem<tauri::Wry>>)> {
    let show = MenuItemBuilder::new("Show Window").id("show").enabled(true).build(app)?;
    let test = MenuItemBuilder::new("Run Test Now").id("test").enabled(true).build(app)?;
    let quit = MenuItemBuilder::new("Quit").id("quit").enabled(true).build(app)?;

    // 为每个配置的模型创建一个结果菜单项（初始显示 "—"）
    let configured = load_models();
    let mut result_items: Vec<MenuItem<tauri::Wry>> = Vec::new();
    for m in configured.iter() {
        let item = MenuItemBuilder::new(format!("{}: —", truncate(&m.name, 20)))
            .enabled(true)
            .build(app)?;
        result_items.push(item);
    }

    // 用 MenuBuilder 逐项添加，避免手工拼接 &dyn 切片导致 macOS 上漏显
    let mut builder = MenuBuilder::new(app).item(&show).item(&test);
    for item in &result_items {
        builder = builder.item(item);
    }
    builder = builder.item(&quit);
    let menu = builder.build()?;

    Ok((menu, result_items))
}

/// 重建托盘：删除旧 tray 并用新菜单重建。
/// 在启动时以及模型配置变更后调用，确保所有模型都显示。
fn rebuild_tray_menu(app: &AppHandle) -> tauri::Result<()> {
    // 删除旧托盘（连同其菜单）
    let _ = app.remove_tray_by_id("main-tray");

    let (menu, result_items) = build_tray_menu(app)?;

    // 把结果项存进全局 state，供 update_tray_results 按 index 更新文本
    {
        let tray_state = app.state::<TrayState>();
        *tray_state.result_items.lock().unwrap() = result_items;
    }

    TrayIconBuilder::with_id("main-tray")
        .icon(app.default_window_icon().unwrap().clone())
        .menu(&menu)
        .tooltip("Model Speed Tester")
        .on_menu_event(handle_tray_menu_event)
        .build(app)?;

    Ok(())
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let t: String = s.chars().take(max).collect();
        format!("{}…", t)
    }
}

fn main() {
    let models = load_models();
    let settings = load_settings();

    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_notification::init())
        .manage(AppState {
            models: Mutex::new(models),
            settings: Mutex::new(settings),
        })
        .manage(TrayState {
            result_items: std::sync::Mutex::new(Vec::new()),
        })
        .setup(|app| {
            // 构建初始托盘菜单（为每个模型创建结果项）
            rebuild_tray_menu(app.handle())?;
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            get_models,
            save_models_cmd,
            get_settings,
            save_settings_cmd,
            get_history,
            export_history,
            clear_history,
            run_test,
            is_test_running,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
