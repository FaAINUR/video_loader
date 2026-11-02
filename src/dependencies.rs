//! Диагностика окружения. Установка выполняется пользователем в терминале.
use std::fs::{self, OpenOptions};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::thread;
use std::time::{Duration, Instant};

#[derive(Clone, Debug, PartialEq)]
pub enum State {
    Ready,
    Missing,
    Broken,
    Timeout,
    Unverified,
}
#[derive(Clone, Debug)]
pub struct Entry {
    pub name: String,
    pub path: Option<PathBuf>,
    pub state: State,
    pub detail: String,
}
#[derive(Clone, Debug)]
pub struct Report {
    pub entries: Vec<Entry>,
    pub arch: bool,
}
impl Report {
    pub fn needs_attention(&self) -> bool {
        self.entries.iter().any(|e| e.state != State::Ready)
    }
    pub fn packages(&self) -> Vec<&'static str> {
        let bad = |name: &str| {
            self.entries
                .iter()
                .any(|e| e.name == name && e.state != State::Ready)
        };
        let mut result = Vec::new();
        if bad("yt-dlp") {
            result.push("yt-dlp");
        }
        if bad("FFmpeg") || bad("FFprobe") {
            result.push("ffmpeg");
        }
        if bad("Deno") {
            result.push("deno");
        }
        if bad("yt-dlp-ejs") {
            result.push("yt-dlp-ejs");
        }
        result
    }
}
pub fn arch_release(contents: &str) -> bool {
    contents
        .lines()
        .filter_map(|line| line.split_once('='))
        .filter(|(key, _)| *key == "ID" || *key == "ID_LIKE")
        .any(|(_, value)| {
            value
                .trim_matches(|c| c == '\'' || c == '"')
                .split_whitespace()
                .any(|word| word == "arch")
        })
}
fn executable(path: &Path) -> bool {
    if !path.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        return fs::metadata(path)
            .map(|m| m.permissions().mode() & 0o111 != 0)
            .unwrap_or(false);
    }
    #[cfg(not(unix))]
    {
        true
    }
}
pub fn locate(program: &str) -> Option<PathBuf> {
    let paths = std::env::var_os("PATH")?;
    for directory in std::env::split_paths(&paths) {
        let path = directory.join(program);
        if executable(&path) {
            return Some(path);
        }
        #[cfg(windows)]
        {
            let exe = directory.join(format!("{program}.exe"));
            if executable(&exe) {
                return Some(exe);
            }
        }
    }
    None
}

struct Capture(PathBuf);
impl Drop for Capture {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}
static CAPTURE_ID: AtomicU64 = AtomicU64::new(0);

fn run_probe(program: &Path, args: &[&str], name: &str) -> Entry {
    let mut entry = Entry {
        name: name.into(),
        path: Some(program.into()),
        state: State::Broken,
        detail: String::new(),
    };
    let result = (|| -> Result<(), String> {
        let path = std::env::temp_dir().join(format!(
            "video-loader-check-{}-{}-{}.log",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos(),
            CAPTURE_ID.fetch_add(1, Ordering::Relaxed)
        ));
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let file = options.open(&path).map_err(|e| e.to_string())?;
        let capture = Capture(path);
        let mut command = Command::new(program);
        command
            .args(args)
            .env("LC_ALL", "C")
            .stdin(Stdio::null())
            .stdout(Stdio::from(file.try_clone().map_err(|e| e.to_string())?))
            .stderr(Stdio::from(file));
        crate::platform::hide_console(&mut command);
        let mut child = command.spawn().map_err(|e| e.to_string())?;
        let start = Instant::now();
        let status = loop {
            match child.try_wait() {
                Ok(Some(status)) => break Some(status),
                Ok(None) if start.elapsed() < Duration::from_secs(8) => {
                    thread::sleep(Duration::from_millis(50))
                }
                Ok(None) => {
                    let _ = child.kill();
                    let _ = child.wait();
                    break None;
                }
                Err(e) => {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(e.to_string());
                }
            }
        };
        let mut bytes = Vec::new();
        if let Ok(file) = fs::File::open(&capture.0) {
            let _ = file.take(16384).read_to_end(&mut bytes);
        }
        let output = String::from_utf8_lossy(&bytes);
        entry.detail = output
            .lines()
            .find(|line| !line.trim().is_empty())
            .unwrap_or("Нет вывода")
            .chars()
            .take(500)
            .collect();
        entry.state = match status {
            Some(s) if s.success() => State::Ready,
            Some(s) => {
                entry.detail = format!("{s}: {}", entry.detail);
                State::Broken
            }
            None => {
                entry.detail = "Нет ответа за 8 секунд".into();
                State::Timeout
            }
        };
        Ok(())
    })();
    if let Err(error) = result {
        entry.detail = error;
    }
    entry
}
fn check_program(name: &str, program: &str, args: &[&str]) -> Entry {
    match locate(program) {
        Some(path) => run_probe(&path, args, name),
        None => Entry {
            name: name.into(),
            path: None,
            state: State::Missing,
            detail: "Не найден в PATH".into(),
        },
    }
}
pub fn check() -> Report {
    let arch = cfg!(target_os = "linux")
        && fs::read_to_string("/etc/os-release")
            .map(|s| arch_release(&s))
            .unwrap_or(false);
    let mut entries = vec![
        check_program("yt-dlp", "yt-dlp", &["--version"]),
        check_program("FFmpeg", "ffmpeg", &["-version"]),
        check_program("FFprobe", "ffprobe", &["-version"]),
        check_program("Deno", "deno", &["--version"]),
    ];
    entries.push(if arch && Path::new("/usr/bin/pacman").is_file() {
        let mut entry = run_probe(Path::new("/usr/bin/pacman"), &["-Q", "yt-dlp-ejs"], "yt-dlp-ejs");
        if entry.state == State::Broken && entry.detail.contains("was not found") { entry.state = State::Missing; }
        entry
    } else {
        Entry { name: "yt-dlp-ejs".into(), path: None, state: State::Unverified,
            detail: "Пакетная проверка доступна только в Arch. При другой установке yt-dlp компонент нужно проверить отдельно.".into() }
    });
    Report { entries, arch }
}

/// Только фиксированный список пакетов: никакого пользовательского текста в shell.
pub fn install_command(packages: &[&str]) -> Result<String, String> {
    if packages.is_empty() {
        return Err("Нет пакетов для установки.".into());
    }
    if packages
        .iter()
        .any(|p| !["yt-dlp", "ffmpeg", "deno", "yt-dlp-ejs"].contains(p))
    {
        return Err("Недопустимое имя пакета.".into());
    }
    Ok(format!(
        "sudo /usr/bin/pacman -Syu --needed {}",
        packages.join(" ")
    ))
}
pub fn install_in_terminal(packages: &[&str]) -> Result<(), String> {
    if !fs::read_to_string("/etc/os-release")
        .map(|s| arch_release(&s))
        .unwrap_or(false)
        || !cfg!(target_os = "linux")
    {
        return Err("Установка поддерживается только для Arch / Omarchy.".into());
    }
    let install = install_command(packages)?;
    let script = format!("{install}; result=$?; printf '\\nКод завершения установки: %s\\nНажмите Enter, чтобы закрыть окно.\\n' \"$result\"; read -r answer; exit \"$result\"");
    let mut command = if let Some(path) = locate("alacritty") {
        let mut c = Command::new(path);
        c.args(["-e", "/bin/sh", "-c", &script]);
        c
    } else if let Some(path) = locate("kitty") {
        let mut c = Command::new(path);
        c.args(["/bin/sh", "-c", &script]);
        c
    } else if let Some(path) = locate("foot") {
        let mut c = Command::new(path);
        c.args(["/bin/sh", "-c", &script]);
        c
    } else if let Some(path) = locate("xterm") {
        let mut c = Command::new(path);
        c.args(["-e", "/bin/sh", "-c", &script]);
        c
    } else {
        return Err(
            "Поддерживаемый терминал не найден. Скопируйте команду и выполните её вручную.".into(),
        );
    };
    let status = command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map_err(|e| format!("Не удалось открыть терминал: {e}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!(
            "Терминал завершился с кодом {status}. Проверьте окружение повторно."
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn arch_detection() {
        assert!(arch_release("ID=arch\n"));
        assert!(arch_release("ID=other\nID_LIKE=\"arch linux\""));
        assert!(!arch_release("ID=debian\n"));
        assert!(!arch_release("NAME=Arch Linux\nID=notarch"));
    }
    #[test]
    fn command_is_explicit() {
        assert_eq!(
            install_command(&["yt-dlp", "ffmpeg"]).unwrap(),
            "sudo /usr/bin/pacman -Syu --needed yt-dlp ffmpeg"
        );
    }
    #[test]
    fn rejects_shell_input() {
        assert!(install_command(&["ffmpeg; touch /tmp/bad"]).is_err());
        assert!(install_command(&[]).is_err());
    }
    #[test]
    fn package_selection_deduplicates_ffmpeg() {
        let entries = ["yt-dlp", "FFmpeg", "FFprobe", "Deno", "yt-dlp-ejs"]
            .iter()
            .map(|n| Entry {
                name: (*n).into(),
                path: None,
                state: State::Missing,
                detail: String::new(),
            })
            .collect();
        assert_eq!(
            Report {
                entries,
                arch: true
            }
            .packages(),
            vec!["yt-dlp", "ffmpeg", "deno", "yt-dlp-ejs"]
        );
    }
    #[test]
    fn healthy_report_has_no_installation() {
        let report = Report {
            entries: vec![Entry {
                name: "yt-dlp".into(),
                path: None,
                state: State::Ready,
                detail: "1".into(),
            }],
            arch: true,
        };
        assert!(report.packages().is_empty());
        assert!(!report.needs_attention());
    }
    #[test]
    fn timeout_needs_attention() {
        let report = Report {
            entries: vec![Entry {
                name: "Deno".into(),
                path: None,
                state: State::Timeout,
                detail: String::new(),
            }],
            arch: true,
        };
        assert!(report.needs_attention());
        assert_eq!(report.packages(), vec!["deno"]);
    }
}
