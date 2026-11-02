//! Особенности операционных систем: процессы, консоль, открытие папок.

use std::path::Path;
use std::process::{Command, Stdio};
use std::thread;

pub fn hide_console(command: &mut Command) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }

    #[cfg(not(windows))]
    {
        let _ = command;
    }
}

pub fn prepare_process(command: &mut Command) {
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }

    hide_console(command);
}

pub fn kill_process_tree(process_id: u32) -> Result<(), String> {
    #[cfg(unix)]
    let result = Command::new("kill")
        .arg("-KILL")
        .arg("--")
        .arg(format!("-{process_id}"))
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();

    #[cfg(windows)]
    let result = {
        let mut command = Command::new("taskkill");
        hide_console(&mut command);

        command
            .arg("/PID")
            .arg(process_id.to_string())
            .args(["/T", "/F"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
    };

    #[cfg(any(unix, windows))]
    {
        let status = result.map_err(|error| format!("Не удалось остановить загрузку: {error}"))?;

        if status.success() {
            Ok(())
        } else {
            Err("Не удалось завершить дерево процессов загрузки.".to_owned())
        }
    }

    #[cfg(not(any(unix, windows)))]
    {
        let _ = process_id;
        Err("Остановка не поддерживается на этой ОС.".to_owned())
    }
}

#[cfg(target_os = "linux")]
fn opener() -> Option<Command> {
    Some(Command::new("xdg-open"))
}

#[cfg(target_os = "windows")]
fn opener() -> Option<Command> {
    Some(Command::new("explorer.exe"))
}

#[cfg(not(any(target_os = "linux", target_os = "windows")))]
fn opener() -> Option<Command> {
    None
}

pub fn open_directory(path: &Path) -> Result<(), String> {
    std::fs::create_dir_all(path).map_err(|error| format!("Не удалось создать папку: {error}"))?;

    let Some(mut command) = opener() else {
        return Err("Открытие папки не поддерживается на этой ОС.".to_owned());
    };

    let mut child = command
        .arg(path)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|error| format!("Не удалось открыть папку: {error}"))?;

    thread::spawn(move || {
        let _ = child.wait();
    });

    Ok(())
}
