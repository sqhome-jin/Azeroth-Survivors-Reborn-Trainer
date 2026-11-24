use std::{
    fmt,
    fs::{self, OpenOptions},
    io::{self, Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
    sync::{
        Arc, OnceLock,
        atomic::{AtomicUsize, Ordering},
        mpsc::{self, Receiver, SyncSender},
    },
    thread::{self, JoinHandle},
};

const LOG_LIMIT: u64 = 10_000;

enum Message {
    Event(&'static str, String),
    Stop,
}

#[derive(Clone)]
struct Sender {
    channel: SyncSender<Message>,
    dropped: Arc<AtomicUsize>,
}

impl Sender {
    fn write(&self, name: &'static str, details: fmt::Arguments<'_>) {
        if let Err(mpsc::TrySendError::Full(_)) = self
            .channel
            .try_send(Message::Event(name, details.to_string()))
        {
            self.dropped.fetch_add(1, Ordering::Relaxed);
        }
    }
}

pub struct Log {
    sender: Sender,
    background: Option<JoinHandle<io::Result<()>>>,
}

static SENDER: OnceLock<Sender> = OnceLock::new();

impl Log {
    pub fn start(path: PathBuf) -> io::Result<Self> {
        let log = Self::new(path)?;
        let _ = SENDER.set(log.sender.clone());
        Ok(log)
    }

    fn new(path: PathBuf) -> io::Result<Self> {
        let (channel, receiver) = mpsc::sync_channel(512);
        let dropped = Arc::new(AtomicUsize::new(0));
        let writer_dropped = dropped.clone();
        let background = thread::Builder::new()
            .name("reborn-log-writer".into())
            .spawn(move || write_file(path, receiver, writer_dropped))?;
        Ok(Self {
            sender: Sender { channel, dropped },
            background: Some(background),
        })
    }
}

impl Drop for Log {
    fn drop(&mut self) {
        let _ = self.sender.channel.send(Message::Stop);
        if let Some(background) = self.background.take() {
            let _ = background.join();
        }
    }
}

pub fn event(name: &'static str, details: fmt::Arguments<'_>) {
    if let Some(sender) = SENDER.get() {
        sender.write(name, details);
    }
}

pub fn recent_lines(path: &Path) -> io::Result<String> {
    const READ_LIMIT: u64 = 24 * 1024;
    const LINE_LIMIT: usize = 40;
    let mut file = fs::File::open(path)?;
    let start = file.metadata()?.len().saturating_sub(READ_LIMIT);
    file.seek(SeekFrom::Start(start))?;
    let mut bytes = Vec::new();
    file.take(READ_LIMIT).read_to_end(&mut bytes)?;
    let contents = String::from_utf8_lossy(&bytes);
    let contents = if start == 0 {
        contents.as_ref()
    } else {
        contents.split_once('\n').map_or("", |(_, tail)| tail)
    };
    let mut lines: Vec<_> = contents.lines().rev().take(LINE_LIMIT).collect();
    lines.reverse();
    Ok(lines.join("\n"))
}

#[cfg(windows)]
fn timestamp() -> String {
    use windows_sys::Win32::{Foundation::SYSTEMTIME, System::SystemInformation::GetSystemTime};

    let mut utc = SYSTEMTIME::default();
    // Windows populates the provided SYSTEMTIME with a UTC timestamp.
    unsafe { GetSystemTime(&mut utc) };
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}.{:03}Z",
        utc.wYear, utc.wMonth, utc.wDay, utc.wHour, utc.wMinute, utc.wSecond, utc.wMilliseconds,
    )
}

#[cfg(not(windows))]
fn timestamp() -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    let seconds = now.as_secs() as libc::time_t;
    let mut utc: libc::tm = unsafe { std::mem::zeroed() };
    // gmtime_r writes the broken-down UTC time into the provided struct.
    unsafe { libc::gmtime_r(&seconds, &mut utc) };
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}.{:03}Z",
        utc.tm_year + 1900,
        utc.tm_mon + 1,
        utc.tm_mday,
        utc.tm_hour,
        utc.tm_min,
        utc.tm_sec,
        now.subsec_millis(),
    )
}

fn write_file(
    path: PathBuf,
    receiver: Receiver<Message>,
    dropped: Arc<AtomicUsize>,
) -> io::Result<()> {
    if let Some(directory) = path.parent() {
        fs::create_dir_all(directory)?;
    }
    let lock = OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(path.with_extension("lock"))?;
    let pid = std::process::id();
    let append = |name, details: &str| -> io::Result<()> {
        let details = details.replace('\r', "\\r").replace('\n', "\\n");
        let mut line = format!("{} pid={pid} {name} {details}\n", timestamp());
        const TRUNCATED: &str = " [truncated]\n";
        if line.len() > LOG_LIMIT as usize {
            let end = line.floor_char_boundary(LOG_LIMIT as usize - TRUNCATED.len());
            line.truncate(end);
            line.push_str(TRUNCATED);
        }
        lock.lock()?;
        let result = append_line(&path, line.as_bytes(), LOG_LIMIT);
        result.and(lock.unlock())
    };

    while let Ok(message) = receiver.recv() {
        let skipped = dropped.swap(0, Ordering::Relaxed);
        if skipped > 0 {
            append("log.records_dropped", &format!("count={skipped}"))?;
        }
        match message {
            Message::Event(name, details) => append(name, &details)?,
            Message::Stop => break,
        }
    }
    Ok(())
}

fn append_line(path: &Path, line: &[u8], limit: u64) -> io::Result<()> {
    match fs::metadata(path) {
        Ok(metadata) if metadata.len() + line.len() as u64 > limit => {
            fs::rename(path, path.with_extension("previous.log"))?;
        }
        Ok(_) => {}
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(error),
    }
    OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?
        .write_all(line)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    #[test]
    fn keeps_only_the_latest_previous_log() {
        let path = std::env::temp_dir().join(format!(
            "reborn-log-rotation-test-{}.log",
            std::process::id()
        ));
        let previous = path.with_extension("previous.log");
        append_line(&path, b"first\n", 8).unwrap();
        append_line(&path, b"second\n", 8).unwrap();
        append_line(&path, b"third\n", 8).unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"third\n");
        assert_eq!(fs::read(&previous).unwrap(), b"second\n");
        fs::remove_file(path).unwrap();
        fs::remove_file(previous).unwrap();
    }

    #[test]
    fn appends_single_lines_that_are_visible_while_running() {
        let path =
            std::env::temp_dir().join(format!("reborn-rust-test-{}.log", std::process::id()));
        fs::write(&path, "previous session\n").unwrap();
        let log = Log::new(path.clone()).unwrap();
        log.sender.write("hunt.start", format_args!("a\nb"));
        let deadline = Instant::now() + Duration::from_secs(3);
        loop {
            let data = fs::read_to_string(&path).unwrap();
            if data.contains("hunt.start a\\nb\n") {
                assert!(data.starts_with("previous session\n"));
                assert_eq!(data.lines().count(), 2);
                break;
            }
            assert!(Instant::now() < deadline, "event never became visible");
            thread::sleep(Duration::from_millis(10));
        }
        log.sender.write("app.exit", format_args!("ok"));
        drop(log);
        assert!(fs::read_to_string(&path).unwrap().contains("app.exit ok"));
        fs::remove_file(path.with_extension("lock")).unwrap();
        fs::remove_file(path).unwrap();
    }
}
