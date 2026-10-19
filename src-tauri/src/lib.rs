use animation_replace_roblox::{
    LogCallback, StudioParser, scan_windows_credential_for_cookie,
};
use roboat::assetdelivery::AssetBatchResponse;
use roboat::catalog::AssetType;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::fmt;
use std::sync::{Arc, LazyLock};
use std::time::Duration;
use tauri::{AppHandle, Emitter};

static GUI_HTTP_CLIENT: LazyLock<reqwest::Client> = LazyLock::new(|| {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(15))
        .pool_max_idle_per_host(10)
        .build()
        .unwrap_or_default()
});

#[derive(Serialize, Deserialize)]
pub struct ReplacementConfig {
    pub cookie: String,
    pub api_key: String,
    pub file: String,
    pub output: Option<String>,
    pub group: Option<u64>,
    pub threads: Option<u64>,
    pub unformatted_ids: bool,
    pub dry_run: bool,
    pub keep_names: bool,
}

impl fmt::Debug for ReplacementConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ReplacementConfig")
            .field("cookie", &"[REDACTED]")
            .field("api_key", &"[REDACTED]")
            .field("file", &self.file)
            .field("output", &self.output)
            .field("group", &self.group)
            .field("threads", &self.threads)
            .field("unformatted_ids", &self.unformatted_ids)
            .field("dry_run", &self.dry_run)
            .field("keep_names", &self.keep_names)
            .finish()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LogEvent {
    pub level: String,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProgressEvent {
    pub current: usize,
    pub total: usize,
    pub stage: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ReplacementResult {
    pub success: bool,
    pub animations_count: usize,
    pub audios_count: usize,
    pub replaced_count: usize,
    pub message: String,
}

fn emit_log(app: &AppHandle, level: &str, msg: impl Into<String>) {
    let _ = app.emit(
        "app_log",
        LogEvent {
            level: level.to_string(),
            message: msg.into(),
        },
    );
}

fn emit_progress(app: &AppHandle, current: usize, total: usize, stage: &str) {
    let _ = app.emit(
        "app_progress",
        ProgressEvent {
            current,
            total,
            stage: stage.to_string(),
        },
    );
}

#[tauri::command]
async fn pick_file() -> Result<Option<String>, String> {
    let file = rfd::AsyncFileDialog::new()
        .add_filter("Roblox Place Files (*.rbxl, *.rbxlx)", &["rbxl", "rbxlx"])
        .add_filter("All Files (*.*)", &["*"])
        .set_title("Select Roblox Place File")
        .pick_file()
        .await;

    Ok(file.map(|f| f.path().to_string_lossy().to_string()))
}

#[tauri::command]
async fn pick_output_file(default_name: Option<String>) -> Result<Option<String>, String> {
    let mut dialog = rfd::AsyncFileDialog::new()
        .add_filter("Roblox Place Files (*.rbxl, *.rbxlx)", &["rbxl", "rbxlx"])
        .set_title("Select Save Location");

    if let Some(name) = default_name {
        dialog = dialog.set_file_name(&name);
    }

    let file = dialog.save_file().await;
    Ok(file.map(|f| f.path().to_string_lossy().to_string()))
}

#[tauri::command]
async fn run_replacement(
    app: AppHandle,
    mut config: ReplacementConfig,
) -> Result<ReplacementResult, String> {
    emit_log(&app, "info", "Starting animation replacement process...");
    emit_progress(&app, 0, 100, "Initializing");

    if config.cookie.trim().is_empty() {
        emit_log(
            &app,
            "info",
            "Auto-Mode: Detecting .ROBLOSECURITY cookie from Roblox Studio...",
        );
        if let Some(detected) = scan_windows_credential_for_cookie() {
            config.cookie = detected;
            emit_log(
                &app,
                "info",
                "Successfully retrieved cookie from Roblox Studio session!",
            );
        } else {
            return Err("No cookie provided and could not auto-detect Roblox Studio session. Please log in to Roblox Studio or paste your cookie.".to_string());
        }
    }

    let file_path = shellexpand::tilde(&config.file).to_string();

    let builder = StudioParser::builder()
        .file_path(&file_path)
        .api_key(&config.api_key)
        .roblosecurity(&config.cookie);

    emit_log(&app, "info", format!("Parsing place file: {}", file_path));
    emit_progress(&app, 10, 100, "Reading Place File");

    let mut parser = builder
        .build()
        .map_err(|e| format!("Failed to load place file: {}", e))?;

    let mut seen_ids: HashSet<String> = HashSet::new();
    let mut all_animations: Vec<AssetBatchResponse> = Vec::new();
    let mut all_audios: Vec<AssetBatchResponse> = Vec::new();

    // 1. Scan Workspace animations
    emit_log(&app, "info", "Scanning Animation instances in Workspace...");
    emit_progress(&app, 25, 100, "Scanning Workspace");

    match parser.workspace_animations().await {
        Ok(animations) => {
            for asset in animations {
                if let Some(ref asset_id) = asset.request_id {
                    if !seen_ids.insert(asset_id.clone()) {
                        continue;
                    }
                    let is_anim = matches!(asset.asset_type, Some(AssetType::Animation))
                        || asset.asset_type_id == Some(24);
                    let is_audio = matches!(asset.asset_type, Some(AssetType::Audio))
                        || asset.asset_type_id == Some(3);

                    if is_anim || (!is_audio && asset.asset_type.is_none()) {
                        all_animations.push(asset);
                    } else if is_audio {
                        all_audios.push(asset);
                    }
                }
            }
        }
        Err(e) => {
            emit_log(&app, "warn", format!("Failed scanning workspace: {}", e));
        }
    }

    // 2. Scan Script animations
    emit_log(
        &app,
        "info",
        format!(
            "Scanning Scripts (unformatted: {})...",
            config.unformatted_ids
        ),
    );
    emit_progress(&app, 40, 100, "Scanning Scripts");

    match parser
        .all_animations_in_scripts(config.unformatted_ids)
        .await
    {
        Ok(animations) => {
            for asset in animations {
                if let Some(ref asset_id) = asset.request_id {
                    if !seen_ids.insert(asset_id.clone()) {
                        continue;
                    }
                    let is_anim = matches!(asset.asset_type, Some(AssetType::Animation))
                        || asset.asset_type_id == Some(24);
                    let is_audio = matches!(asset.asset_type, Some(AssetType::Audio))
                        || asset.asset_type_id == Some(3);

                    if is_anim {
                        all_animations.push(asset);
                    } else if is_audio {
                        all_audios.push(asset);
                    }
                }
            }
        }
        Err(e) => {
            emit_log(&app, "warn", format!("Failed scanning scripts: {}", e));
        }
    }

    let anim_count = all_animations.len();
    let audio_count = all_audios.len();
    emit_log(
        &app,
        "info",
        format!(
            "Found {} unique animations and {} audios",
            anim_count, audio_count
        ),
    );

    if config.dry_run {
        emit_log(
            &app,
            "info",
            "Dry-run mode is enabled. No assets will be reuploaded or saved.",
        );
        for anim in &all_animations {
            if let Some(id) = &anim.request_id {
                emit_log(&app, "info", format!("- Asset ID: {}", id));
            }
        }
        emit_progress(&app, 100, 100, "Completed (Dry Run)");
        return Ok(ReplacementResult {
            success: true,
            animations_count: anim_count,
            audios_count: audio_count,
            replaced_count: 0,
            message: format!("Dry run finished. Found {} animations.", anim_count),
        });
    }

    if anim_count == 0 {
        emit_log(&app, "info", "No animations found to replace.");
        emit_progress(&app, 100, 100, "Completed");
        return Ok(ReplacementResult {
            success: true,
            animations_count: 0,
            audios_count: audio_count,
            replaced_count: 0,
            message: "No animations were found in the place file.".to_string(),
        });
    }

    // 3. Upload animations (reusing shared uploader & caches from StudioParser)
    emit_log(
        &app,
        "info",
        format!(
            "Starting reupload of {} animations (Threads: {})...",
            anim_count,
            config.threads.unwrap_or(10)
        ),
    );
    emit_progress(&app, 50, 100, "Reuploading Assets");

    let uploader = parser
        .animation_uploader()
        .map_err(|e| format!("Uploader initialization error: {}", e))?;

    let app_for_cb = app.clone();
    let progress_cb: animation_replace_roblox::ProgressCallback = Arc::new(
        move |done: usize, total: usize, old_id: &str, new_id: &str| {
            let scaled = 50 + ((done * 35) / total.max(1));
            emit_progress(&app_for_cb, scaled, 100, "Reuploading Assets");
            emit_log(
                &app_for_cb,
                "info",
                format!(
                    "Uploaded animation {} -> {} ({}/{})",
                    old_id, new_id, done, total
                ),
            );
        },
    );

    let app_for_log = app.clone();
    let log_cb: LogCallback = Arc::new(move |level: &str, message: &str| {
        emit_log(&app_for_log, level, message.to_string());
    });

    let is_open_cloud = !uploader.api_key.trim().is_empty();

    let mapping = uploader
        .reupload_all_assets_with_callbacks(
            all_animations,
            config.group,
            config.threads,
            config.keep_names,
            Some(progress_cb),
            Some(log_cb),
        )
        .await
        .map_err(|e| {
            let mode = if is_open_cloud {
                "Open Cloud API Mode"
            } else {
                "Studio Cookie Mode"
            };
            let group_note = if let Some(gid) = config.group {
                format!(" [Target Group: {}]", gid)
            } else {
                String::new()
            };
            let advice = if config.group.is_some() {
                " Check that: 1) Your account has 'Create and edit group items' permission in this group. 2) If using an Open Cloud API key, ensure the key was created under the Group in Creator Hub with 'Assets (Write)' permissions."
            } else {
                " Check that your .ROBLOSECURITY cookie is valid and not expired, or that your API key has 'Assets (Write)' permissions."
            };
            format!("Upload error [{}]{}: {}.{}", mode, group_note, e, advice)
        })?;

    let replaced_count = mapping.len();
    if replaced_count == 0 {
        return Err("No animations were successfully uploaded.".to_string());
    }

    emit_log(
        &app,
        "info",
        format!("Successfully reuploaded {} animations", replaced_count),
    );

    // 4. Update Place & Scripts
    emit_log(
        &app,
        "info",
        "Updating Script references & Animation instances...",
    );
    emit_progress(&app, 85, 100, "Updating Place Data");

    parser.update_script_animations(&mapping, config.unformatted_ids);
    parser.update_game_animations(&mapping);

    // 5. Save file atomically
    let target_save = config
        .output
        .as_deref()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or(&file_path);
    emit_log(
        &app,
        "info",
        format!("Saving modified place to: {}", target_save),
    );
    emit_progress(&app, 95, 100, "Saving Place File");

    parser
        .save_to_rbxl(target_save)
        .map_err(|e| format!("Failed to save place file: {}", e))?;

    emit_log(&app, "info", "Done! Place successfully saved.");
    emit_progress(&app, 100, 100, "Completed Successfully");

    Ok(ReplacementResult {
        success: true,
        animations_count: anim_count,
        audios_count: audio_count,
        replaced_count,
        message: format!(
            "Successfully replaced {} animation(s) and saved to '{}'",
            replaced_count, target_save
        ),
    })
}

#[derive(Serialize, Deserialize)]
pub struct DetectedAccount {
    pub found: bool,
    pub cookie: String,
    pub user_id: Option<u64>,
    pub username: Option<String>,
    pub display_name: Option<String>,
    pub message: String,
}

impl fmt::Debug for DetectedAccount {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DetectedAccount")
            .field("found", &self.found)
            .field("cookie", &"[REDACTED]")
            .field("user_id", &self.user_id)
            .field("username", &self.username)
            .field("display_name", &self.display_name)
            .field("message", &self.message)
            .finish()
    }
}

#[tauri::command]
async fn auto_detect_cookie() -> Result<DetectedAccount, String> {
    let Some(cookie) = scan_windows_credential_for_cookie() else {
        return Ok(DetectedAccount {
            found: false,
            cookie: String::new(),
            user_id: None,
            username: None,
            display_name: None,
            message: "No Roblox Studio credentials found in Windows Credential Manager. Please make sure you are logged into Roblox Studio.".to_string(),
        });
    };

    // Verify cookie and fetch authenticated user info from Roblox API
    let resp = GUI_HTTP_CLIENT
        .get("https://users.roblox.com/v1/users/authenticated")
        .header("Cookie", format!(".ROBLOSECURITY={}", cookie))
        .send()
        .await;

    if let Ok(response) = resp
        && response.status().is_success()
        && let Ok(json) = response.json::<serde_json::Value>().await
    {
        let user_id = json.get("id").and_then(|v| v.as_u64());
        let username = json
            .get("name")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        let display_name = json
            .get("displayName")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        return Ok(DetectedAccount {
            found: true,
            cookie,
            user_id,
            username: username.clone(),
            display_name,
            message: format!(
                "Successfully detected Roblox Studio login for: {}",
                username.unwrap_or_else(|| "Roblox User".to_string())
            ),
        });
    }

    // Cookie found but couldn't verify online (or offline)
    Ok(DetectedAccount {
        found: true,
        cookie,
        user_id: None,
        username: None,
        display_name: None,
        message: "Roblox Studio cookie was detected from Windows Credential Manager.".to_string(),
    })
}

#[derive(Serialize, Deserialize)]
pub struct GeneratedApiKeyResult {
    pub created: bool,
    pub api_key: String,
    pub message: String,
}

impl fmt::Debug for GeneratedApiKeyResult {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("GeneratedApiKeyResult")
            .field("created", &self.created)
            .field("api_key", &"[REDACTED]")
            .field("message", &self.message)
            .finish()
    }
}

#[tauri::command]
async fn auto_generate_api_key(cookie: String) -> Result<GeneratedApiKeyResult, String> {
    let active_cookie = if cookie.trim().is_empty() {
        scan_windows_credential_for_cookie().ok_or_else(|| {
            "Could not find Roblox Studio cookie. Please detect or enter your cookie first."
                .to_string()
        })?
    } else {
        cookie.trim().to_string()
    };

    let cookie_header = format!(".ROBLOSECURITY={}", active_cookie);

    let payload = serde_json::json!({
        "cloudAuthUserConfiguredProperties": {
            "name": format!("AnimationReplacerAuto_{}", std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs() % 100000),
            "description": "Auto-generated key for Roblox Animation Replacer",
            "isEnabled": true,
            "allowedCidrs": ["0.0.0.0/0"],
            "scopes": [
                {
                    "scopeType": "asset",
                    "targetParts": ["U"],
                    "operations": ["read", "write"]
                }
            ]
        }
    });

    let mut xcsrf = String::new();
    for _ in 0..2 {
        let mut req = GUI_HTTP_CLIENT
            .post("https://apis.roblox.com/cloud-authentication/v1/apiKey")
            .header("Cookie", &cookie_header)
            .header("Content-Type", "application/json");

        if !xcsrf.is_empty() {
            req = req.header("x-csrf-token", &xcsrf);
        }

        let resp = req.json(&payload).send().await.map_err(|e| e.to_string())?;

        if resp.status().as_u16() == 403 {
            if let Some(token) = resp
                .headers()
                .get("x-csrf-token")
                .and_then(|v| v.to_str().ok())
                && xcsrf.is_empty()
            {
                xcsrf = token.to_string();
                continue;
            }
            return Ok(GeneratedApiKeyResult {
                created: false,
                api_key: String::new(),
                message: "Roblox requires 2FA verification to create a Cloud API Key. Leaving API Key empty will automatically use Direct Studio Cookie Upload instead!".to_string(),
            });
        }

        if resp.status().is_success()
            && let Ok(json) = resp.json::<serde_json::Value>().await
            && let Some(secret) = json.get("apikeySecret").and_then(|v| v.as_str())
        {
            return Ok(GeneratedApiKeyResult {
                created: true,
                api_key: secret.to_string(),
                message: "Successfully generated Open Cloud API Key automatically!".to_string(),
            });
        }

        break;
    }

    Ok(GeneratedApiKeyResult {
        created: false,
        api_key: String::new(),
        message: "Using Direct Studio Cookie Upload mode (No API Key required).".to_string(),
    })
}

#[tauri::command]
async fn open_browser_url(url: String) -> Result<(), String> {
    let parsed = reqwest::Url::parse(&url).map_err(|e| format!("Invalid URL: {}", e))?;
    if parsed.scheme() != "https" {
        return Err("Only HTTPS URLs are allowed.".to_string());
    }
    let allowed_host = parsed.host_str().is_some_and(|host| {
        host == "roblox.com"
            || host.ends_with(".roblox.com")
            || host == "github.com"
            || host.ends_with(".github.com")
    });
    if !allowed_host {
        return Err("URL domain is not in the allowed list.".to_string());
    }
    open::that(parsed.as_str()).map_err(|e| e.to_string())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![
            pick_file,
            pick_output_file,
            run_replacement,
            auto_detect_cookie,
            auto_generate_api_key,
            open_browser_url
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
