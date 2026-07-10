use std::fs;
use std::io;
use std::process::Command;

/// Return whether a process id currently accepts signal checks and is not a zombie.
pub(crate) fn process_alive(pid: u32) -> bool {
    if !process_accepts_signal(pid) {
        return false;
    }

    match linux_process_state(pid) {
        Ok(Some('Z')) => false,
        Ok(_) => true,
        Err(error) if error.kind() == io::ErrorKind::NotFound => false,
        Err(_) => true,
    }
}

/// Send a signal to a process id.
pub(crate) fn signal_process(pid: u32, signal: &str) -> bool {
    Command::new("kill")
        .arg(format!("-{signal}"))
        .arg(pid.to_string())
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

fn process_accepts_signal(pid: u32) -> bool {
    Command::new("kill")
        .arg("-0")
        .arg(pid.to_string())
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

fn linux_process_state(pid: u32) -> io::Result<Option<char>> {
    let status = fs::read_to_string(format!("/proc/{pid}/status"))?;
    Ok(parse_linux_process_state(&status))
}

fn parse_linux_process_state(status: &str) -> Option<char> {
    status
        .lines()
        .find_map(|line| line.strip_prefix("State:"))
        .and_then(|state| state.trim_start().chars().next())
}

#[cfg(test)]
mod tests {
    use super::parse_linux_process_state;

    #[test]
    fn parses_linux_process_state_code() {
        let status = "Name:\tllama-server\nState:\tS (sleeping)\nPid:\t123\n";

        assert_eq!(parse_linux_process_state(status), Some('S'));
    }

    #[test]
    fn parses_linux_zombie_process_state_code() {
        let status = "Name:\tllama-server\nState:\tZ (zombie)\nPid:\t123\n";

        assert_eq!(parse_linux_process_state(status), Some('Z'));
    }

    #[test]
    fn returns_none_when_linux_process_state_is_missing() {
        let status = "Name:\tllama-server\nPid:\t123\n";

        assert_eq!(parse_linux_process_state(status), None);
    }
}
