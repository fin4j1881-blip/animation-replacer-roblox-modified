use animation_replace_roblox::{StudioParser, scan_windows_credential_for_cookie};
use clap::Parser;
use roboat::assetdelivery::AssetBatchResponse;
use roboat::catalog::AssetType;
use std::collections::HashSet;
use std::fmt;
use std::fs;
use std::path::Path;
use tracing::{error, info, warn};
use tracing_subscriber::{EnvFilter, fmt as tracing_fmt, prelude::*};

#[derive(Parser)]
struct Args {
    /// .ROBLOSECURITY cookie string [Optional on Windows - auto-detected from Roblox Studio if omitted]
    #[arg(long, short, default_value = "")]
    cookie: String,

    /// Open cloud API Key [optional - uses Roblox Studio cookie upload if omitted]
    #[arg(long, short, default_value = "")]
    api_key: String,

    /// file PATH of the .rbxl / .rbxlx file [REQUIRED]
    #[arg(long, short)]
    file: String,

    /// Save the copy instead replacing file [AVOID DATA LOSS]
    #[arg(long, short)]
    output: Option<String>,

    /// Required if the game will be published to a Group [Id of the group]
    #[arg(long, short)]
    group: Option<u64>,

    /// How many concurrent tasks using semaphore. [defaulted to 50]
    #[arg(long, short)]
    threads: Option<u64>,

    /// Search every big digits in scripts if flag is active (instead of just scanning for rbxasset)
    #[arg(long, short)]
    unformatted_ids: bool,

    /// Scan and report what would change without uploading or saving anything.
    #[arg(long, short)]
    dry_run: bool,

    /// Reuploaded animations keep the original asset names (one extra API call
    /// per animation).
    #[arg(long, short)]
    keep_names: bool,
}

impl fmt::Debug for Args {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Args")
            .field("cookie", &"[REDACTED]")
            .field(
                "api_key",
                if self.api_key.is_empty() {
                    &"[EMPTY]"
                } else {
                    &"[REDACTED]"
                },
            )
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

fn cleanup_old_logs(log_dir: &str, prefix: &str, keep: usize) {
    let dir = Path::new(log_dir);

    if !dir.exists() {
        return;
    }

    let Ok(read_dir) = fs::read_dir(dir) else {
        return;
    };

    let mut entries: Vec<_> = read_dir
        .filter_map(|e| e.ok())
        .filter(|e| e.file_name().to_string_lossy().starts_with(prefix))
        .collect();

    entries.sort_by_key(|e| {
        e.metadata()
            .and_then(|m| m.modified())
            .unwrap_or(std::time::SystemTime::UNIX_EPOCH)
    });

    if entries.len() > keep {
        for entry in &entries[..entries.len() - keep] {
            if let Err(e) = fs::remove_file(entry.path()) {
                warn!("Failed to delete old log {:?}: {}", entry.path(), e);
            }
        }
    }
}

fn categorize_assets(
    batch: Vec<AssetBatchResponse>,
    seen_ids: &mut HashSet<String>,
    all_animations: &mut Vec<AssetBatchResponse>,
    all_audios: &mut Vec<AssetBatchResponse>,
) {
    for asset in batch {
        if let Some(ref asset_id) = asset.request_id {
            if !seen_ids.insert(asset_id.clone()) {
                continue;
            }

            match asset.asset_type {
                Some(AssetType::Animation) => all_animations.push(asset),
                Some(AssetType::Audio) => all_audios.push(asset),
                _ => {}
            }
        }
    }
}

#[tokio::main]
async fn main() {
    // Clean up old logs, keeping only the 5 most recent
    cleanup_old_logs("logs", "app.log", 5);

    let file_appender = tracing_appender::rolling::hourly("logs", "app.log");
    let (non_blocking, _guard) = tracing_appender::non_blocking(file_appender);

    // Log to both stdout and file
    tracing_subscriber::registry()
        // stdout: info and above
        .with(
            tracing_fmt::layer()
                .with_writer(std::io::stdout)
                .with_target(false)
                .with_thread_ids(false)
                .with_file(false)
                .with_line_number(false)
                .with_filter(EnvFilter::new("info")),
        )
        // file: debug and above
        .with(
            tracing_fmt::layer()
                .with_writer(non_blocking)
                .with_ansi(false)
                .with_filter(EnvFilter::new("debug")),
        )
        .init();

    let mut args = Args::parse();

    if args.cookie.trim().is_empty() {
        info!("No --cookie provided; attempting to auto-detect Roblox Studio session...");
        if let Some(detected) = scan_windows_credential_for_cookie() {
            args.cookie = detected;
            info!("Successfully auto-detected .ROBLOSECURITY cookie from Roblox Studio!");
        } else {
            error!(
                "No .ROBLOSECURITY cookie provided and could not auto-detect from Roblox Studio. Please pass --cookie <COOKIE> or log into Roblox Studio."
            );
            return;
        }
    }

    let file_path = shellexpand::tilde(&args.file).to_string();
    let mut seen_ids: HashSet<String> = HashSet::new();

    // Build the parser with the shared roboat client
    let builder = StudioParser::builder()
        .file_path(&file_path)
        .api_key(&args.api_key)
        .roblosecurity(&args.cookie);

    let mut parser = match builder.build() {
        Ok(parser) => parser,
        Err(e) => {
            error!("Error loading file: {}", e);
            return;
        }
    };

    let mut all_animations: Vec<AssetBatchResponse> = Vec::new();
    let mut all_audios: Vec<AssetBatchResponse> = Vec::new();

    match parser.workspace_animations().await {
        Ok(animations) => {
            categorize_assets(
                animations,
                &mut seen_ids,
                &mut all_animations,
                &mut all_audios,
            );
        }
        Err(e) => {
            warn!("Failed to fetch workspace animations: {:?}", e);
        }
    }

    match parser.all_animations_in_scripts(args.unformatted_ids).await {
        Ok(animations) => {
            categorize_assets(
                animations,
                &mut seen_ids,
                &mut all_animations,
                &mut all_audios,
            );
        }
        Err(e) => {
            warn!("Failed to fetch script animations: {:?}", e);
        }
    }

    info!(
        "Total Animations fetched from game: {}",
        all_animations.len()
    );
    info!("Total Audios fetched from game: {}", all_audios.len());

    if args.dry_run {
        info!("Dry-run mode enabled: no uploads or file changes will be made.");
        info!("Would reupload {} animation(s).", all_animations.len());
        for animation in &all_animations {
            if let Some(id) = &animation.request_id {
                info!("  - Asset ID: {} (type: {:?})", id, animation.asset_type);
            }
        }
        return;
    }

    if all_animations.is_empty() {
        info!("No animations found to replace. Exiting without modifying file.");
        return;
    }

    let Ok(uploader) = parser.animation_uploader() else {
        error!("Invalid authentication credentials.");
        return;
    };

    match uploader
        .reupload_all_assets(all_animations, args.group, args.threads, args.keep_names)
        .await
    {
        Ok(animation_mapping) => {
            if animation_mapping.is_empty() {
                warn!("No animations were successfully uploaded; skipping file save.");
                return;
            }
            parser.update_script_animations(&animation_mapping, args.unformatted_ids);
            parser.update_game_animations(&animation_mapping);

            let target_path = args.output.as_deref().unwrap_or(&file_path);
            if let Err(e) = parser.save_to_rbxl(target_path) {
                error!("Failed to save output to {}: {}", target_path, e);
            } else {
                info!(
                    "Successfully replaced {} animations and saved place file to {}",
                    animation_mapping.len(),
                    target_path
                );
            }
        }
        Err(e) => {
            error!("Failed to upload animations: {:?}", e);
        }
    }
}
