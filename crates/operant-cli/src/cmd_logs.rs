use std::fs;
use std::io::{BufRead, BufReader, Seek, SeekFrom};
use std::path::PathBuf;

use anyhow::{Context, Result};
use clap::Subcommand;
use operant_core::config::AppConfig;

#[derive(Debug, Clone, Subcommand)]
pub enum LogsSubcommand {
    /// Show recent log entries
    Show {
        /// Number of lines to show
        #[arg(default_value = "50")]
        lines: usize,
        /// Filter by log level (e.g. INFO, WARN, ERROR)
        #[arg(long)]
        level: Option<String>,
        /// Filter by component
        #[arg(long)]
        component: Option<String>,
    },
    /// Follow log output (tail -f style)
    Follow {
        /// Filter by log level
        #[arg(long)]
        level: Option<String>,
    },
}

pub async fn handle_logs_command(config: &AppConfig, cmd: LogsSubcommand) -> Result<()> {
    match cmd {
        LogsSubcommand::Show {
            lines,
            level,
            component,
        } => cmd_show(config, lines, level.as_deref(), component.as_deref()).await,
        LogsSubcommand::Follow { level } => cmd_follow(config, level.as_deref()).await,
    }
}

fn log_path(config: &AppConfig) -> Option<PathBuf> {
    if let Some(path) = &config.logging.log_file {
        return Some(PathBuf::from(path));
    }
    let default = operant_core::platform::operant_data_dir().join("operant.log");
    if default.exists() {
        return Some(default);
    }
    None
}

async fn cmd_show(
    config: &AppConfig,
    lines: usize,
    level: Option<&str>,
    component: Option<&str>,
) -> Result<()> {
    let path = log_path(config)
        .context("No log file found. Configure logging.log_file or run Operant first.")?;
    let file =
        fs::File::open(&path).with_context(|| format!("Failed to open log: {}", path.display()))?;
    let reader = BufReader::new(file);
    let all_lines: Vec<String> = reader.lines().map_while(Result::ok).collect();
    let slice: Vec<&String> = all_lines
        .iter()
        .filter(|l| {
            let level_ok = level.is_none_or(|lv| l.contains(lv));
            let comp_ok = component.is_none_or(|c| l.contains(c));
            level_ok && comp_ok
        })
        .collect();
    let start = if slice.len() > lines {
        slice.len() - lines
    } else {
        0
    };
    for line in &slice[start..] {
        println!("{}", line);
    }
    Ok(())
}

/// The follower's position in the log file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct FollowPosition {
    /// The byte offset to read from.
    offset: u64,
    /// The file's identity, so a REPLACEMENT is detected.
    ///
    /// A size comparison is not enough: the log rotation (iter-991) renames
    /// the live file aside and starts a new one, and the new file can be
    /// LARGER than the old one's length was (the old file was 11 bytes, the
    /// new one is 15), so `current_size < last_size` is false and the
    /// follower seeks into the middle of the new file and prints half a
    /// line. The inode is the signal that the file was REPLACED.
    #[cfg(unix)]
    inode: u64,
    /// The file's identity on platforms without an inode.
    #[cfg(not(unix))]
    identity: (u64, u64),
}

/// The identity of the file at `path`.
fn file_identity(path: &std::path::Path) -> Result<FollowPosition> {
    let meta =
        fs::metadata(path).with_context(|| format!("Failed to stat log: {}", path.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        Ok(FollowPosition {
            offset: 0,
            inode: meta.ino(),
        })
    }
    #[cfg(not(unix))]
    {
        let created = meta
            .created()
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_nanos() as u64)
            .unwrap_or(0);
        Ok(FollowPosition {
            offset: 0,
            identity: (meta.len(), created),
        })
    }
}

/// Read the log lines appended since the last poll, and return the new
/// position.
///
/// Returns the lines to print and the position to continue from.
///
/// A file that was REPLACED (the rotation renames the live file aside and
/// starts a new one) resets the offset to zero, so the follower reads the
/// new file from its start. Without that, the follower tracked a byte
/// offset into the OLD file: `current_size > last_size` could stay false
/// for a long time — so it printed NOTHING until 32 MB of new output had
/// accumulated — and then it seeked into a stale offset and printed half a
/// line.
///
/// This is a function, not a block inside the follow loop, so a test can
/// drive it: the loop itself never returns.
fn read_appended(
    path: &std::path::Path,
    position: FollowPosition,
    level: Option<&str>,
) -> Result<(Vec<String>, FollowPosition)> {
    let current_size = fs::metadata(path)
        .with_context(|| format!("Failed to stat log: {}", path.display()))?
        .len();
    let current = file_identity(path)?;

    // A DIFFERENT file: the rotation replaced it. Read from the start.
    #[cfg(unix)]
    let replaced = current.inode != position.inode;
    #[cfg(not(unix))]
    let replaced = current.identity != position.identity;

    let offset = if replaced { 0 } else { position.offset };

    if current_size <= offset {
        return Ok((
            Vec::new(),
            FollowPosition {
                offset: current_size,
                ..current
            },
        ));
    }

    let mut file =
        fs::File::open(path).with_context(|| format!("Failed to open log: {}", path.display()))?;
    file.seek(SeekFrom::Start(offset))?;
    let reader = BufReader::new(file);
    let lines: Vec<String> = reader
        .lines()
        .map_while(Result::ok)
        .filter(|line| level.is_none_or(|lv| line.contains(lv)))
        .collect();
    Ok((
        lines,
        FollowPosition {
            offset: current_size,
            ..current
        },
    ))
}

async fn cmd_follow(config: &AppConfig, level: Option<&str>) -> Result<()> {
    let path = log_path(config).context("No log file found.")?;
    let mut position = file_identity(&path)?;
    position.offset = fs::metadata(&path)?.len();
    println!("Following: {} (Ctrl+C to stop)", path.display());
    loop {
        let (lines, new_position) = read_appended(&path, position, level)?;
        for line in lines {
            println!("{}", line);
        }
        position = new_position;
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_log(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "operant_logs_{name}_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        std::fs::create_dir_all(&dir).expect("create temp dir");
        dir.join("gateway.log")
    }

    /// The follower must keep working across a log ROTATION.
    ///
    /// The rotation (iter-991) renames the live file aside and starts a
    /// new one, so the live file becomes a NEW inode whose length starts
    /// near zero. The follower tracked a byte offset into the OLD file:
    /// `current_size > last_size` stayed false, so it printed NOTHING
    /// until 32 MB of new output had accumulated — and then it seeked into
    /// a stale offset. This is the regression iter-991 introduced, and it
    /// broke `operant logs follow` for the owner.
    #[test]
    fn the_follower_reads_the_new_file_after_a_rotation() {
        let path = temp_log("rotate");
        std::fs::write(&path, b"first line\n").expect("write the first file");

        // Read what is there, as the follower does on its first poll.
        let (lines, position) =
            read_appended(&path, file_identity(&path).expect("id"), None).expect("first read");
        assert_eq!(lines, vec!["first line".to_string()]);

        // The rotation: the live file is renamed aside and a new one
        // starts. The new file is LONGER than the old one was, so a size
        // comparison alone would NOT notice the replacement — the inode
        // does.
        let rotated = path.with_extension("log.1");
        std::fs::rename(&path, &rotated).expect("rotate the log");
        std::fs::write(&path, b"after rotation\n").expect("write the new file");
        assert!(
            fs::metadata(&path).expect("stat").len() > position.offset,
            "the new file must be LONGER than the old offset, so a size check alone would miss the replacement"
        );

        let (lines, new_position) =
            read_appended(&path, position, None).expect("read after rotation");
        assert_eq!(
            lines,
            vec!["after rotation".to_string()],
            "the follower must read the NEW file, not seek into it at a stale offset"
        );
        assert_eq!(
            new_position.offset, 15,
            "the offset must track the new file"
        );

        let _ = std::fs::remove_dir_all(path.parent().unwrap_or(&path));
    }

    /// The follower must not RE-READ what it has already printed. That is
    /// the whole point of the offset.
    #[test]
    fn the_follower_does_not_reread_what_it_already_printed() {
        let path = temp_log("offset");
        std::fs::write(&path, b"one\n").expect("write");

        let (lines, position) =
            read_appended(&path, file_identity(&path).expect("id"), None).expect("first read");
        assert_eq!(lines, vec!["one".to_string()]);

        // Nothing new: no lines, and the offset holds.
        let (lines, position2) = read_appended(&path, position, None).expect("second read");
        assert!(lines.is_empty(), "nothing was appended, so nothing to read");
        assert_eq!(
            position2.offset, position.offset,
            "the offset must not move"
        );

        // Something new: only the new line.
        std::fs::write(&path, b"one\ntwo\n").expect("append");
        let (lines, position3) = read_appended(&path, position2, None).expect("third read");
        assert_eq!(lines, vec!["two".to_string()], "only the new line");
        assert_eq!(position3.offset, 8);

        let _ = std::fs::remove_dir_all(path.parent().unwrap_or(&path));
    }
}
