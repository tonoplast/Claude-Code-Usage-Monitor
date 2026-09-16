use std::io::Read;
use std::os::windows::io::FromRawHandle;

use windows::Win32::Storage::FileSystem::WriteFile;
use windows::Win32::System::Console::{GetConsoleMode, GetStdHandle, STD_INPUT_HANDLE, STD_OUTPUT_HANDLE};

fn read_stdin_bytes() -> Vec<u8> {
    unsafe {
        let Ok(handle) = GetStdHandle(STD_INPUT_HANDLE) else {
            return Vec::new();
        };
        if handle.is_invalid() || handle.0.is_null() {
            return Vec::new();
        }
        // GetConsoleMode succeeds on a real console handle and fails on a pipe/file.
        // If stdin is a console (inherited TTY) Claude Code will not send JSON through
        // it, so skip reading to avoid blocking on keyboard input.
        let mut console_mode = windows::Win32::System::Console::CONSOLE_MODE(0);
        if GetConsoleMode(handle, &mut console_mode).is_ok() {
            return Vec::new();
        }
        // stdin is a pipe — Claude Code writes the JSON payload in one shot.
        // Single read() returns as soon as data is in the buffer without waiting for EOF.
        let mut file = std::fs::File::from_raw_handle(handle.0);
        let mut buf = vec![0u8; 65536];
        let n = file.read(&mut buf).unwrap_or(0);
        buf.truncate(n);
        std::mem::forget(file);
        buf
    }
}

fn write_stdout(text: &str) {
    unsafe {
        let Ok(handle) = GetStdHandle(STD_OUTPUT_HANDLE) else { return; };
        if handle.is_invalid() || handle.0.is_null() { return; }
        let mut bytes = text.as_bytes().to_vec();
        bytes.push(b'\n');
        let _ = WriteFile(handle, Some(&bytes), None, None);
    }
}

/// Resolve the account label to show.
///
/// Three detection methods in priority order:
///
/// 1. `--work-cwd <prefix>`: if `workspace.cwd` in the payload starts with
///    the prefix, return "[WORK]"; otherwise "[PERSONAL]". Most reliable
///    because `workspace.cwd` is always present in the payload.
///
/// 2. `--personal-email <email>`: compares against the account email field
///    if the payload ever gains one. Currently falls through to the
///    `CLAUDE_CONFIG_DIR` env-var check: unset or equal to `~/.claude` →
///    "[PERSONAL]", any other explicit path → "[WORK]".
///
/// 3. `--label <label>`: verbatim static label (empty string if absent).
fn resolve_label(args: &[String], payload: &serde_json::Value) -> String {
    // Method 1: workspace cwd prefix (uses confirmed payload field).
    let work_cwd = args
        .windows(2)
        .find(|pair| pair[0] == "--work-cwd")
        .map(|pair| pair[1].to_lowercase().replace('\\', "/"));
    if let Some(work_prefix) = work_cwd {
        let cwd_norm = payload["workspace"]["cwd"]
            .as_str()
            .unwrap_or("")
            .to_lowercase()
            .replace('\\', "/");
        return if cwd_norm.starts_with(&work_prefix) {
            "[WORK]".to_string()
        } else {
            "[PERSONAL]".to_string()
        };
    }

    // Method 2: account email (future payload support) with CLAUDE_CONFIG_DIR fallback.
    let personal_email = args
        .windows(2)
        .find(|pair| pair[0] == "--personal-email")
        .map(|pair| pair[1].to_lowercase());
    if let Some(personal_email) = personal_email {
        for field in &["email", "emailAddress", "email_address"] {
            if let Some(addr) = payload["account"][field].as_str() {
                return if addr.to_lowercase() == personal_email {
                    "[PERSONAL]".to_string()
                } else {
                    "[WORK]".to_string()
                };
            }
        }
        let config_dir = std::env::var("CLAUDE_CONFIG_DIR").ok();
        let default_dir = dirs::home_dir()
            .map(|h| h.join(".claude"))
            .map(|p| p.to_string_lossy().to_lowercase());
        return match config_dir {
            None => "[PERSONAL]".to_string(),
            Some(dir) => {
                if default_dir.as_deref() == Some(dir.to_lowercase().as_str()) {
                    "[PERSONAL]".to_string()
                } else {
                    "[WORK]".to_string()
                }
            }
        };
    }

    // Method 3: static label.
    args.windows(2)
        .find(|pair| pair[0] == "--label")
        .map(|pair| pair[1].to_string())
        .unwrap_or_default()
}

pub fn run(args: &[String]) {
    // Claude Code re-invokes the statusline command itself on every refresh,
    // so this just needs to print one line for the current payload and exit.
    let raw = read_stdin_bytes();
    let stdin = String::from_utf8_lossy(&raw);
    let payload: serde_json::Value =
        serde_json::from_str(&stdin).unwrap_or(serde_json::Value::Null);

    let label = resolve_label(args, &payload);

    let stats_parts = format_statusline_parts(&payload);
    let mut parts: Vec<String> = Vec::new();
    if !label.is_empty() {
        parts.push(label);
    }
    parts.extend(stats_parts);
    if parts.is_empty() {
        parts.push("claude-monitor".to_string());
    }

    write_stdout(&parts.join(" \u{00B7} "));
}

fn clean_pct(value: f64) -> Option<f64> {
    if !value.is_finite() || value < 0.0 {
        return None;
    }
    // Epoch-sized values are a known Claude Code bug where resets_at leaks into
    // used_percentage. Drop them rather than clamp to 100%.
    if value > 101.0 {
        return None;
    }
    Some(value.min(100.0))
}

fn time_remaining(reset_epoch: u64) -> Option<String> {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .ok()?
        .as_secs();
    let secs = reset_epoch.checked_sub(now)?;
    let h = secs / 3600;
    let m = (secs % 3600) / 60;
    Some(if h >= 24 {
        let d = h / 24;
        let h_rem = h % 24;
        if h_rem > 0 { format!("{d}d{h_rem}h") } else { format!("{d}d") }
    } else if h > 0 {
        if m > 0 { format!("{h}h{m}m") } else { format!("{h}h") }
    } else if m > 0 {
        format!("{m}m")
    } else {
        "<1m".to_string()
    })
}

fn format_statusline_parts(payload: &serde_json::Value) -> Vec<String> {
    let mut parts: Vec<String> = Vec::new();

    if let Some(name) = payload["model"]["display_name"].as_str() {
        let name = name.strip_prefix("Claude ").unwrap_or(name);
        if !name.is_empty() {
            parts.push(name.to_string());
        }
    }

    let rate_limits = &payload["rate_limits"];
    for (label, key) in [("5h", "five_hour"), ("7d", "seven_day")] {
        let window = &rate_limits[key];
        let Some(used) = window["used_percentage"].as_f64().and_then(clean_pct) else {
            continue;
        };
        let left = (100.0 - used).max(0.0);
        let mut inner = format!("\u{2193}{left:.0}%");

        if let Some(epoch) = window["resets_at"].as_f64() {
            let epoch = epoch as u64;
            if (946_684_800..=4_102_444_800).contains(&epoch) {
                if let Some(rem) = time_remaining(epoch) {
                    inner.push_str(&format!(" \u{21BB} {rem}"));
                }
            }
        }

        parts.push(format!("{label} [{inner}]"));
    }

    parts
}
