use std::{
    io::SeekFrom,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

use anyhow::{anyhow, bail, Result};
use log::{debug, error, info, warn};
use notify::{
    event::{MetadataKind, ModifyKind, RenameMode},
    EventKind, RecursiveMode, Watcher,
};
use regex::Regex;
use tokio::{
    fs::{File, OpenOptions},
    io::{AsyncReadExt, AsyncSeekExt},
    process::Command,
    sync::{mpsc, mpsc::Receiver},
    time::sleep,
};
use toml::Table;

pub struct Monitor {
    name: String,
    log_regex: Regex,
    exec: String,
    log_file_path: PathBuf,
    log_file: File,
    cursor: u64,
    watcher: Box<dyn Watcher>,
    event_rx: Receiver<Result<notify::Event, notify::Error>>,
}

impl Monitor {
    pub async fn new(name: String, config: Table) -> Result<Self> {
        // Use multi-line regex in case more than one line is read at a time.
        // FIXME: Multi-line mode does not handle carriage returns. Rewrite for Windows support.
        let log_regex_str = format!("(?m){}", config["match_log"].as_str().unwrap());
        let log_regex = Regex::new(&log_regex_str)
            .map_err(|err| anyhow!("Monitor {name}: Failed to parse match_log: {err}"))?;

        let log_file_name = config["log"].as_str().unwrap();
        let log_file_path = Path::new(log_file_name).to_owned();
        let mut log_file = OpenOptions::new()
            .read(true)
            .open(&log_file_path)
            .await
            .map_err(|err| anyhow!("[{name}] Failed to open {log_file_path:?}: {err}"))?;
        log_file.seek(SeekFrom::End(0)).await?;
        let cursor = log_file.stream_position().await?;

        let (event_tx, event_rx) = mpsc::channel(1);
        let mut watcher = notify::recommended_watcher(move |res| {
            event_tx.blocking_send(res).unwrap();
        })?;
        watcher.watch(&log_file_path, RecursiveMode::NonRecursive)?;

        Ok(Self {
            name,
            log_regex,
            exec: config["exec"].as_str().unwrap().to_owned(),
            log_file_path,
            log_file,
            cursor,
            watcher: Box::new(watcher),
            event_rx,
        })
    }

    pub async fn start(&mut self) -> Result<()> {
        info!("Starting monitor `{}`", self.name);

        while let Some(res) = self.event_rx.recv().await {
            match res {
                Ok(event) => self.process_event(event).await?,
                Err(err) => {
                    error!("[{}] Event error: {err}", self.name);
                }
            };
        }

        bail!("[{}] Monitor exited early.", self.name);
    }

    async fn process_event(&mut self, event: notify::Event) -> Result<()> {
        debug!("Event: {:?}", event);

        // Handle move from and deletion. Untested on kernels other than Linux.
        // TODO: Test on other platforms.
        match event.kind {
            EventKind::Modify(ModifyKind::Name(RenameMode::From))
            | EventKind::Modify(ModifyKind::Metadata(MetadataKind::Any)) => {
                self.reinit_file_descriptors().await?;
            }
            _ => {}
        }

        let new_size = self.log_file.metadata().await?.len();
        if new_size < self.cursor {
            warn!("File {:?} was truncated", self.log_file_path);
            self.cursor = new_size;
            return Ok(());
        } else if new_size == self.cursor {
            return Ok(());
        }
        self.process_chunk(new_size).await?;

        Ok(())
    }

    async fn reinit_file_descriptors(&mut self) -> Result<()> {
        info!(
            "File {:?} was renamed. Reestablishing file descriptors.",
            self.log_file_path
        );

        // Handle log rotation.
        // FIXME: Are there any cases where new log files are not generated immediately
        // after rotation?
        self.watcher.unwatch(&self.log_file_path).unwrap();
        let timeout = Instant::now().checked_add(Duration::from_secs(1)).unwrap();
        self.log_file = loop {
            match OpenOptions::new()
                .read(true)
                .open(&self.log_file_path)
                .await
            {
                Ok(file) => break file,
                Err(err) => {
                    if Instant::now() > timeout {
                        bail!("File {:?} was moved: {err}", self.log_file_path);
                    } else {
                        sleep(Duration::from_millis(10)).await;
                    }
                }
            }
        };
        self.cursor = 0;
        self.watcher
            .watch(&self.log_file_path, RecursiveMode::NonRecursive)?;
        info!("File descriptors were reestablished.");

        Ok(())
    }

    async fn process_chunk(&mut self, new_size: u64) -> Result<()> {
        let prefix = format!("[{}]", self.name);
        let chunk_size = new_size - self.cursor;
        info!("{prefix} Log file grew by {chunk_size} bytes");

        // Ensure chunk ends with newline.
        // SeekFrom::End is not used because it introduces a race condition if the
        // file grew immediately after the size was checked.
        self.log_file.seek(SeekFrom::Start(new_size - 1)).await?;
        let mut buffer = [0; 1];
        self.log_file.read(&mut buffer).await?;
        if buffer[0] != '\n' as u8 {
            warn!("{prefix} Log chunk does not end in newline.");
            return Ok(());
        }

        // Match chunk against log_regex and execute on each match.
        self.log_file.seek(SeekFrom::Start(self.cursor)).await?;
        // Don't read the final newline.
        let mut buffer = vec![0; chunk_size as usize - 1];
        self.log_file.read_exact(&mut buffer).await?;
        let buffer_str = match String::from_utf8(buffer) {
            Ok(buffer_str) => buffer_str,
            Err(err) => {
                error!("{prefix} Log chunk is not valid UTF-8: {err}");
                self.cursor = new_size;
                return Ok(());
            }
        };
        for captures in self.log_regex.captures_iter(&buffer_str) {
            info!("Match found");
            let mut command = Command::new("sh");
            command.args(&["-c", &self.exec]);
            for capture_name in self
                .log_regex
                .capture_names()
                .filter(Option::is_some)
                .map(|n| n.unwrap())
            {
                if let Some(capture) = captures.name(capture_name) {
                    command.env(capture_name, capture.as_str());
                } else {
                    warn!("{prefix} Capture group `{capture_name}` was not found.");
                }
            }
            command.spawn()?.wait().await?;
        }

        self.cursor = new_size;

        Ok(())
    }
}
