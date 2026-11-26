//! Owns the download list: persistence, queue, concurrency and the scheduler.

use crate::events::Event;
use crate::http::{JobContext, Shared};
use crate::limiter::Limiter;
use crate::segments::SegmentMap;
use crate::types::{DownloadRequest, Settings, Status, Task, MAX_CONNECTIONS};
use anyhow::{bail, Context, Result};
use chrono::{Datelike, Timelike};
use sha2::Digest;
use std::collections::{BTreeMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;
use tokio::runtime::Handle;
use tokio::task::JoinHandle;

struct Entry {
    task: Shared,
    job: Option<JoinHandle<()>>,
    limit: Arc<Limiter>,
    speed: f64,
    last_sample: (Instant, u64),
    /// Set when the current job is stopped; mutes its late saves.
    cancelled: Arc<AtomicBool>,
}

impl Entry {
    fn new(task: Task) -> Self {
        let limit = Arc::new(Limiter::new(task.req.speed_limit));
        let bytes = task.downloaded();
        Self {
            task: Arc::new(Mutex::new(task)),
            job: None,
            limit,
            speed: 0.0,
            last_sample: (Instant::now(), bytes),
            cancelled: Arc::default(),
        }
    }

    fn busy(&self) -> bool {
        self.job.as_ref().is_some_and(|j| !j.is_finished())
    }

    fn status(&self) -> Status {
        self.task.lock().unwrap().status
    }
}

struct State {
    tasks: BTreeMap<u64, Entry>,
    settings: Settings,
    next_id: u64,
    /// Downloads sent by the browser that await confirmation.
    pending: VecDeque<DownloadRequest>,
    /// Finished downloads nobody has collected yet.
    events: VecDeque<Event>,
}

/// Bounds the list when nothing collects events, as in the command line tool.
const MAX_EVENTS: usize = 1000;

/// A read-only copy of a task for display.
pub struct TaskView {
    pub task: Task,
    /// Bytes per second.
    pub speed: f64,
}

struct Store {
    dir: PathBuf,
    /// Serialises writers so a stale checkpoint cannot replace a newer state.
    lock: Mutex<()>,
}

impl Store {
    fn task_file(&self, id: u64) -> PathBuf {
        self.dir.join("tasks").join(format!("{id}.json"))
    }

    fn save_task(&self, task: &Task) {
        self.save_unless(task, &AtomicBool::new(false));
    }

    fn save_unless(&self, task: &Task, cancelled: &AtomicBool) {
        let _guard = self.lock.lock().unwrap();
        if cancelled.load(Ordering::SeqCst) {
            return;
        }
        let result = serde_json::to_vec(task)
            .map_err(anyhow::Error::from)
            .and_then(|d| crate::fsio::write_private(&self.task_file(task.id), &d));
        if let Err(e) = result {
            eprintln!("could not save download {}: {e:#}", task.id);
        }
    }
}

pub struct Manager {
    state: Mutex<State>,
    store: Arc<Store>,
    global_limit: Arc<Limiter>,
    runtime: Handle,
}

pub fn default_data_dir() -> PathBuf {
    if let Some(d) = std::env::var_os("FLUXDM_DATA_DIR") {
        return PathBuf::from(d);
    }
    dirs::data_dir().unwrap_or_else(|| PathBuf::from(".")).join("FluxDM")
}

fn validate_url(url: &str) -> Result<()> {
    let parsed = url::Url::parse(url).context("that is not a valid address")?;
    if !matches!(parsed.scheme(), "http" | "https" | "ftp") {
        bail!("only http, https and ftp addresses are supported");
    }
    if parsed.host_str().is_none_or(str::is_empty) {
        bail!("the address has no host");
    }
    Ok(())
}

fn file_digest<D: Digest>(path: &Path) -> std::io::Result<String> {
    use std::io::Read;
    let mut file = std::fs::File::open(path)?;
    let mut hasher = D::new();
    let mut buf = vec![0u8; 256 * 1024];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hasher.finalize().iter().map(|b| format!("{b:02x}")).collect())
}

pub fn verify_checksum(path: &Path, expected: &str) -> Result<()> {
    let (kind, want) = expected.trim().split_once(':').context("write checksums as sha256:<hex>")?;
    let got = match kind.to_ascii_lowercase().as_str() {
        "sha256" => file_digest::<sha2::Sha256>(path)?,
        "sha1" => file_digest::<sha1::Sha1>(path)?,
        "md5" => file_digest::<md5::Md5>(path)?,
        other => bail!("unknown checksum type {other}"),
    };
    if !got.eq_ignore_ascii_case(want.trim()) {
        bail!("checksum mismatch: file has {kind}:{got}");
    }
    Ok(())
}

async fn run_job(ctx: JobContext, task: Shared) {
    let (url, is_stream) = {
        let t = task.lock().unwrap();
        (t.req.url.clone(), t.parts_total > 0)
    };
    let result = if url.to_ascii_lowercase().starts_with("ftp:") {
        crate::ftp::download(ctx.clone(), task.clone()).await
    } else if is_stream || crate::hls::looks_like_playlist(&url) {
        crate::hls::download(ctx.clone(), task.clone()).await
    } else {
        crate::http::download(ctx.clone(), task.clone()).await
    };
    let result = match result {
        Ok(()) => {
            let (path, sum) = {
                let t = task.lock().unwrap();
                (t.final_path(), t.req.checksum.clone().filter(|c| !c.trim().is_empty()))
            };
            match (path, sum) {
                (Some(p), Some(sum)) => tokio::task::spawn_blocking(move || verify_checksum(&p, &sum))
                    .await
                    .map_err(anyhow::Error::from)
                    .and_then(|r| r),
                _ => Ok(()),
            }
        }
        e => e,
    };
    let snapshot = {
        let mut t = task.lock().unwrap();
        t.segments.reset_runtime();
        match result {
            Ok(()) => {
                t.status = Status::Completed;
                t.completed_at = Some(chrono::Utc::now().timestamp());
                t.error = None;
                // Credentials are not needed once the file is complete.
                t.req.cookie = None;
                t.req.password = None;
            }
            Err(e) => {
                t.status = Status::Failed;
                t.error = Some(format!("{e:#}"));
            }
        }
        t.clone()
    };
    (ctx.save)(&snapshot);
}

impl Manager {
    pub fn open(data_dir: PathBuf, runtime: Handle) -> Result<Arc<Self>> {
        let tasks_dir = data_dir.join("tasks");
        std::fs::create_dir_all(&tasks_dir)
            .with_context(|| format!("create {}", tasks_dir.display()))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&data_dir, std::fs::Permissions::from_mode(0o700))?;
        }
        let settings: Settings = match std::fs::read(data_dir.join("settings.json")) {
            Ok(d) => serde_json::from_slice(&d).context("settings.json is damaged")?,
            Err(_) => Settings::default(),
        };
        let mut tasks = BTreeMap::new();
        for entry in std::fs::read_dir(&tasks_dir)? {
            let path = entry?.path();
            if path.extension().is_none_or(|e| e != "json") {
                continue;
            }
            match std::fs::read(&path).map_err(anyhow::Error::from).and_then(|d| {
                serde_json::from_slice::<Task>(&d).map_err(anyhow::Error::from)
            }) {
                Ok(mut t) => {
                    t.segments.reset_runtime();
                    // Interrupted by a crash or quit: pick up again.
                    if t.status == Status::Downloading {
                        t.status = Status::Queued;
                    }
                    tasks.insert(t.id, Entry::new(t));
                }
                Err(e) => eprintln!("skipping {}: {e:#}", path.display()),
            }
        }
        let next_id = tasks.keys().next_back().map_or(1, |id| id + 1);
        Ok(Arc::new(Self {
            global_limit: Arc::new(Limiter::new(settings.speed_limit)),
            state: Mutex::new(State { tasks, settings, next_id, pending: VecDeque::new(), events: VecDeque::new() }),
            store: Arc::new(Store { dir: data_dir, lock: Mutex::new(()) }),
            runtime,
        }))
    }

    pub fn data_dir(&self) -> &Path {
        &self.store.dir
    }

    pub fn settings(&self) -> Settings {
        self.state.lock().unwrap().settings.clone()
    }

    pub fn update_settings(&self, mut settings: Settings) -> Result<()> {
        settings.max_connections = settings.max_connections.clamp(1, MAX_CONNECTIONS);
        settings.max_active = settings.max_active.clamp(1, 16);
        if !settings.proxy.trim().is_empty() {
            crate::http::build_client(&settings)?;
        }
        crate::fsio::write_private(
            &self.store.dir.join("settings.json"),
            &serde_json::to_vec_pretty(&settings)?,
        )?;
        self.global_limit.set_rate(settings.speed_limit);
        self.state.lock().unwrap().settings = settings;
        Ok(())
    }

    /// Add a download. With `start` false it waits, paused.
    pub fn add(&self, req: DownloadRequest, start: bool) -> Result<u64> {
        validate_url(&req.url)?;
        let mut s = self.state.lock().unwrap();
        let id = s.next_id;
        s.next_id += 1;
        let mut task = Task::new(id, req);
        if !start {
            task.status = Status::Paused;
        }
        self.store.save_task(&task);
        s.tasks.insert(id, Entry::new(task));
        Ok(id)
    }

    /// Queue a download from the browser, respecting the confirmation setting.
    pub fn offer(&self, req: DownloadRequest) -> Result<()> {
        validate_url(&req.url)?;
        let confirm = {
            let mut s = self.state.lock().unwrap();
            let confirm = s.settings.confirm_browser;
            if confirm {
                if s.pending.len() >= 1000 {
                    bail!("too many downloads are waiting for confirmation");
                }
                s.pending.push_back(req.clone());
            }
            confirm
        };
        if !confirm {
            self.add(req, true)?;
        }
        Ok(())
    }

    pub fn pending(&self) -> Option<(DownloadRequest, usize)> {
        let s = self.state.lock().unwrap();
        s.pending.front().map(|r| (r.clone(), s.pending.len()))
    }

    pub fn take_pending(&self) -> Option<DownloadRequest> {
        self.state.lock().unwrap().pending.pop_front()
    }

    /// Downloads that completed or failed since the last call, each once.
    pub fn take_events(&self) -> Vec<Event> {
        self.state.lock().unwrap().events.drain(..).collect()
    }

    pub fn list(&self) -> Vec<TaskView> {
        let s = self.state.lock().unwrap();
        s.tasks
            .values()
            .map(|e| TaskView { task: e.task.lock().unwrap().clone(), speed: e.speed })
            .collect()
    }

    /// True once no job is still running or writing state for this download.
    pub fn is_idle(&self, id: u64) -> bool {
        self.state.lock().unwrap().tasks.get(&id).is_none_or(|e| !e.busy())
    }

    /// Stop a running job and give the entry a fresh task object, so a job that
    /// is still winding down cannot touch the state of the next run.
    fn halt(&self, entry: &mut Entry, status: Status) -> Option<JoinHandle<()>> {
        entry.cancelled.store(true, Ordering::SeqCst);
        let job = entry.job.take();
        if let Some(j) = &job {
            j.abort();
        }
        let mut copy = entry.task.lock().unwrap().clone();
        if copy.status != Status::Completed {
            copy.status = status;
        }
        copy.segments.reset_runtime();
        entry.task = Arc::new(Mutex::new(copy.clone()));
        entry.speed = 0.0;
        self.store.save_task(&copy);
        job
    }

    pub fn pause(&self, id: u64) {
        let mut s = self.state.lock().unwrap();
        if let Some(e) = s.tasks.get_mut(&id) {
            if matches!(e.status(), Status::Queued | Status::Downloading) {
                self.halt(e, Status::Paused);
            }
        }
    }

    /// Start or continue a download now, even outside the scheduler window.
    pub fn resume(&self, id: u64) {
        self.requeue(id, true);
    }

    /// Put a download back in the queue, subject to the scheduler.
    pub fn enqueue(&self, id: u64) {
        self.requeue(id, false);
    }

    fn requeue(&self, id: u64, force: bool) {
        let s = self.state.lock().unwrap();
        if let Some(e) = s.tasks.get(&id) {
            let mut t = e.task.lock().unwrap();
            if matches!(t.status, Status::Paused | Status::Failed | Status::Queued) {
                if t.status != Status::Queued || force {
                    t.status = Status::Queued;
                }
                t.force = force;
                t.error = None;
                self.store.save_task(&t);
            }
        }
    }

    pub fn set_speed_limit(&self, id: u64, rate: u64) {
        let s = self.state.lock().unwrap();
        if let Some(e) = s.tasks.get(&id) {
            e.limit.set_rate(rate);
            let mut t = e.task.lock().unwrap();
            t.req.speed_limit = rate;
            self.store.save_task(&t);
        }
    }

    /// Replace an expired link while keeping downloaded data. The next resume
    /// still verifies size and validators.
    pub fn set_url(&self, id: u64, url: &str) -> Result<()> {
        validate_url(url)?;
        let mut s = self.state.lock().unwrap();
        let e = s.tasks.get_mut(&id).context("no such download")?;
        if e.busy() {
            bail!("pause the download first");
        }
        let mut t = e.task.lock().unwrap();
        t.req.url = url.to_string();
        // A different mirror usually has different validators.
        t.etag = None;
        t.last_modified = None;
        self.store.save_task(&t);
        Ok(())
    }

    fn delete_partials(task: &Task) {
        if let Some(p) = task.part_path() {
            let _ = std::fs::remove_file(p);
        }
        if let Some(d) = crate::hls::parts_dir(task) {
            let _ = std::fs::remove_dir_all(d);
        }
    }

    /// Download again from the beginning.
    pub fn restart(self: &Arc<Self>, id: u64) {
        let mut s = self.state.lock().unwrap();
        let Some(e) = s.tasks.get_mut(&id) else { return };
        let job = self.halt(e, Status::Paused);
        let (task, store) = (e.task.clone(), self.store.clone());
        // Wait for the old job before deleting what it may still be writing.
        self.runtime.spawn(async move {
            if let Some(j) = job {
                let _ = j.await;
            }
            let mut t = task.lock().unwrap();
            Self::delete_partials(&t);
            t.reset_progress();
            t.segments = SegmentMap::default();
            t.completed_at = None;
            t.error = None;
            t.note = None;
            t.status = Status::Queued;
            t.force = true;
            store.save_task(&t);
        });
    }

    /// Remove from the list. Partial data is always deleted; a finished file only
    /// when `delete_file` is set.
    pub fn remove(&self, id: u64, delete_file: bool) {
        let mut s = self.state.lock().unwrap();
        let Some(mut e) = s.tasks.remove(&id) else { return };
        let job = self.halt(&mut e, Status::Paused);
        let (task, store) = (e.task.clone(), self.store.clone());
        self.runtime.spawn(async move {
            if let Some(j) = job {
                let _ = j.await;
            }
            let t = task.lock().unwrap();
            Self::delete_partials(&t);
            if delete_file && t.status == Status::Completed {
                if let Some(p) = t.final_path() {
                    let _ = std::fs::remove_file(p);
                }
            }
            let _guard = store.lock.lock().unwrap();
            let _ = std::fs::remove_file(store.task_file(t.id));
        });
    }

    pub fn pause_all(&self) {
        let ids: Vec<u64> = self.state.lock().unwrap().tasks.keys().copied().collect();
        ids.into_iter().for_each(|id| self.pause(id));
    }

    pub fn resume_all(&self) {
        let ids: Vec<u64> = self.state.lock().unwrap().tasks.keys().copied().collect();
        ids.into_iter().for_each(|id| self.enqueue(id));
    }

    /// Stop jobs but remember them as queued, for a clean exit.
    pub fn shutdown(&self) {
        let mut s = self.state.lock().unwrap();
        for e in s.tasks.values_mut() {
            if e.busy() {
                self.halt(e, Status::Queued);
            }
        }
    }

    pub fn tick(&self) {
        let now = chrono::Local::now();
        let weekday = now.weekday().num_days_from_monday() as usize;
        self.tick_at(weekday, now.hour() * 60 + now.minute());
    }

    /// One scheduling step. Call a few times per second.
    pub fn tick_at(&self, weekday: usize, minute: u32) {
        let mut s = self.state.lock().unwrap();
        let settings = s.settings.clone();
        let open = settings.schedule.allows(weekday, minute);
        let mut running = 0;
        let mut finished = Vec::new();
        for e in s.tasks.values_mut() {
            let (bytes, force) = {
                let t = e.task.lock().unwrap();
                (t.downloaded(), t.force)
            };
            let elapsed = e.last_sample.0.elapsed().as_secs_f64();
            if elapsed >= 0.5 {
                let rate = bytes.saturating_sub(e.last_sample.1) as f64 / elapsed;
                e.speed = if e.speed == 0.0 { rate } else { e.speed * 0.6 + rate * 0.4 };
                e.last_sample = (Instant::now(), bytes);
            }
            if e.job.as_ref().is_some_and(JoinHandle::is_finished) {
                e.job = None;
                // Read the status only now that the job has ended; an earlier
                // copy could predate its result.
                let mut t = e.task.lock().unwrap();
                // A job that panicked never recorded a result.
                if t.status == Status::Downloading {
                    t.status = Status::Failed;
                    t.error = Some("the download stopped unexpectedly".into());
                    self.store.save_task(&t);
                }
                if matches!(t.status, Status::Completed | Status::Failed) {
                    let failed = t.status == Status::Failed;
                    finished.push(Event {
                        id: t.id,
                        name: t.file_name.clone(),
                        path: t.final_path().filter(|_| !failed),
                        dir: t.dir.clone(),
                        error: failed.then(|| t.error.clone().unwrap_or_else(|| "unknown error".into())),
                    });
                }
            }
            if e.busy() {
                if !open && !force {
                    self.halt(e, Status::Queued);
                } else {
                    running += 1;
                }
            } else {
                e.speed = 0.0;
            }
        }
        for event in finished {
            if s.events.len() >= MAX_EVENTS {
                s.events.pop_front();
            }
            s.events.push_back(event);
        }
        for e in s.tasks.values_mut() {
            if running >= settings.max_active {
                break;
            }
            {
                let t = e.task.lock().unwrap();
                if e.busy() || t.status != Status::Queued || !(open || t.force) {
                    continue;
                }
            }
            let snapshot = {
                let mut t = e.task.lock().unwrap();
                t.status = Status::Downloading;
                t.error = None;
                t.clone()
            };
            self.store.save_task(&snapshot);
            let store = self.store.clone();
            let cancelled = Arc::new(AtomicBool::new(false));
            e.cancelled = cancelled.clone();
            let ctx = JobContext {
                settings: settings.clone(),
                global_limit: self.global_limit.clone(),
                task_limit: e.limit.clone(),
                save: Arc::new(move |t: &Task| store.save_unless(t, &cancelled)),
            };
            e.last_sample = (Instant::now(), snapshot.downloaded());
            e.job = Some(self.runtime.spawn(run_job(ctx, e.task.clone())));
            running += 1;
        }
    }
}
