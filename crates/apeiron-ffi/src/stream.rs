//! Writing a file through the C ABI (SDK.md §2.3). Adding a file pulls its
//! bytes from a reader while the C program pushes them, so the adding runs
//! on a thread of its own and the bytes go to it through a small queue.

use std::io::{self, Read};
use std::sync::mpsc::{Receiver, Sender, SyncSender, channel, sync_channel};
use std::thread::{self, JoinHandle};

use api::{AddFileReport, AddFileRequest, Error, ErrorKind, Repository, Result};

/// How many pieces wait for the adding thread; with [`PIECE`], this bounds
/// the memory a writer uses however large the file is.
const QUEUE: usize = 4;
/// The largest piece a write is sent in.
const PIECE: usize = 1 << 20;

enum Message {
    Data(Vec<u8>),
    /// The file is whole. Without it, the queue closing means the writer
    /// was dropped, and adding fails, so that nothing is stored.
    End,
}

/// What the adding thread reads from.
struct Incoming {
    queue: Receiver<Message>,
    current: Vec<u8>,
    pos: usize,
    ended: bool,
    /// Tells the opener that adding got as far as reading.
    started: Option<Sender<()>>,
}

impl Read for Incoming {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if let Some(started) = self.started.take() {
            // The opener is waiting; if it went away, the queue tells below.
            let _ = started.send(());
        }
        while self.pos == self.current.len() {
            if self.ended {
                return Ok(0);
            }
            match self.queue.recv() {
                Ok(Message::Data(data)) => {
                    self.current = data;
                    self.pos = 0;
                }
                Ok(Message::End) => self.ended = true,
                Err(_) => return Err(io::Error::other("the file was not finished")),
            }
        }
        let n = buf.len().min(self.current.len() - self.pos);
        buf[..n].copy_from_slice(&self.current[self.pos..self.pos + n]);
        self.pos += n;
        Ok(n)
    }
}

/// A file being added; dropped before [`FileWriter::finish`], nothing is
/// stored.
pub(crate) struct FileWriter {
    queue: Option<SyncSender<Message>>,
    worker: Option<JoinHandle<Result<AddFileReport>>>,
    /// Once adding failed, every later call gives the same error, so that
    /// no call can look as if the file was stored.
    failed: Option<Error>,
    finished: bool,
}

impl FileWriter {
    /// Starts adding; returns once the repository is ready for bytes, so a
    /// bad request fails here. It waits for writers before it.
    pub(crate) fn open(repo: &Repository, request: AddFileRequest) -> Result<FileWriter> {
        let (queue, incoming) = sync_channel(QUEUE);
        let (started, ready) = channel();
        let repo = repo.clone();
        let worker = thread::Builder::new()
            .name("apeiron-writer".to_owned())
            .spawn(move || {
                let mut reader = Incoming {
                    queue: incoming,
                    current: Vec::new(),
                    pos: 0,
                    ended: false,
                    started: Some(started),
                };
                repo.add_file(&request, &mut reader)
            })
            .map_err(|e| Error::new(ErrorKind::Io, format!("starting a thread: {e}")))?;
        let mut writer = FileWriter {
            queue: Some(queue),
            worker: Some(worker),
            failed: None,
            finished: false,
        };
        match ready.recv() {
            Ok(()) => Ok(writer),
            Err(_) => Err(writer.failure()),
        }
    }

    /// Adds bytes to the file; it waits while the queue is full.
    pub(crate) fn write(&mut self, data: &[u8]) -> Result<()> {
        self.usable()?;
        for piece in data.chunks(PIECE) {
            let sent = match &self.queue {
                Some(queue) => queue.send(Message::Data(piece.to_vec())).is_ok(),
                None => false,
            };
            if !sent {
                return Err(self.failure());
            }
        }
        Ok(())
    }

    /// Ends the file and stores it as a new snapshot.
    pub(crate) fn finish(&mut self) -> Result<AddFileReport> {
        self.usable()?;
        let sent = match &self.queue {
            Some(queue) => queue.send(Message::End).is_ok(),
            None => false,
        };
        if !sent {
            return Err(self.failure());
        }
        match self.join() {
            Ok(report) => {
                self.finished = true;
                self.queue = None;
                Ok(report)
            }
            Err(e) => {
                self.failed = Some(e.clone());
                Err(e)
            }
        }
    }

    fn usable(&self) -> Result<()> {
        if let Some(e) = &self.failed {
            return Err(e.clone());
        }
        if self.finished {
            return Err(Error::new(
                ErrorKind::InvalidInput,
                "the file is already finished",
            ));
        }
        Ok(())
    }

    /// The error that ended the adding thread, which is ending or over.
    fn failure(&mut self) -> Error {
        let e = match self.join() {
            Ok(_) => Error::new(ErrorKind::Internal, "adding ended before the file did"),
            Err(e) => e,
        };
        self.failed = Some(e.clone());
        e
    }

    fn join(&mut self) -> Result<AddFileReport> {
        let Some(worker) = self.worker.take() else {
            return Err(Error::new(ErrorKind::Internal, "adding is already over"));
        };
        worker.join().unwrap_or_else(|_| {
            Err(Error::new(
                ErrorKind::Internal,
                "Apeiron panicked; please report this bug",
            ))
        })
    }
}

impl Drop for FileWriter {
    fn drop(&mut self) {
        // The queue closes without an end, so adding fails and stores nothing.
        self.queue = None;
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}
