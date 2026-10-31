use crate::config::{Job, Repository, ResolvedConfig};
use crate::errors::AppError;
use crate::snapshot::{Snapshot, SnapshotManager};
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyModifiers};
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph};
use ratatui::Terminal;
use std::collections::VecDeque;
use std::io::{self, BufRead, BufReader, Stdout};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const SIDEBAR_ITEMS: [&str; 3] = ["Jobs", "Repositories", "Logs"];
const TICK_MS: u64 = 100;
const MAX_LOG_LINES: usize = 500;
const STATUS_MESSAGE_SECONDS: u64 = 5;
const DAEMON_CHECK_SECONDS: u64 = 5;

// ---------------------------------------------------------------------------
// Log sink
// ---------------------------------------------------------------------------

/// Shared buffer of log lines, shown in the TUI's Logs view.
///
/// The TUI draws into the alternate screen, so anything written to the
/// terminal's stderr lands in the middle of the rendered widgets. Tracing
/// output is therefore routed here instead of to stderr.
#[derive(Clone, Default)]
struct LogSink {
    state: Arc<Mutex<LogState>>,
}

#[derive(Default)]
struct LogState {
    lines: VecDeque<String>,
    /// Partial trailing line, held until its newline arrives.
    pending: String,
}

impl LogSink {
    fn push(&self, line: String) {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if state.lines.len() >= MAX_LOG_LINES {
            state.lines.pop_front();
        }
        state.lines.push_back(line);
    }

    /// Appends raw bytes, splitting them into complete lines.
    fn write(&self, bytes: &[u8]) {
        let text = String::from_utf8_lossy(bytes);
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        state.pending.push_str(&text);
        while let Some(idx) = state.pending.find('\n') {
            let line: String = state.pending.drain(..=idx).collect();
            let line = line.trim_end().to_string();
            if !line.is_empty() {
                if state.lines.len() >= MAX_LOG_LINES {
                    state.lines.pop_front();
                }
                state.lines.push_back(line);
            }
        }
    }

    /// The most recent lines, oldest first.
    fn tail(&self, count: usize) -> Vec<String> {
        let state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let start = state.lines.len().saturating_sub(count);
        state.lines.iter().skip(start).cloned().collect()
    }
}

/// A `tracing` writer that appends to the TUI's log buffer.
#[derive(Clone)]
struct LogWriter {
    sink: LogSink,
}

impl io::Write for LogWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.sink.write(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for LogWriter {
    type Writer = LogWriter;

    fn make_writer(&'a self) -> Self::Writer {
        self.clone()
    }
}

// ---------------------------------------------------------------------------
// App state
// ---------------------------------------------------------------------------

struct App {
    sidebar_state: ListState,
    jobs: Vec<String>,
    repos: Vec<String>,
    snapshots: Vec<Snapshot>,
    selected_job: Option<String>,
    selected_repo: Option<String>,
    job_list_index: usize,
    repo_list_index: usize,
    job_details: Option<Job>,
    repo_details: Option<Repository>,
    logs: LogSink,
    /// Configuration problems found while loading, shown in the UI.
    problems: Vec<String>,
    status_message: Option<(String, Instant)>,
    running_label: Arc<Mutex<Option<String>>>,
    child_pids: Arc<Mutex<Vec<u32>>>,
    data_loaded: DataLoaded,
    daemon_running: bool,
    last_daemon_check: Instant,
    show_help: bool,
}

struct DataLoaded {
    jobs: bool,
    repos: bool,
    snapshots: bool,
}

impl App {
    fn new(logs: LogSink, problems: Vec<String>) -> Self {
        let mut sidebar_state = ListState::default();
        sidebar_state.select(Some(0));
        Self {
            sidebar_state,
            jobs: Vec::new(),
            repos: Vec::new(),
            snapshots: Vec::new(),
            selected_job: None,
            selected_repo: None,
            job_list_index: 0,
            repo_list_index: 0,
            job_details: None,
            repo_details: None,
            logs,
            problems,
            status_message: None,
            running_label: Arc::new(Mutex::new(None)),
            child_pids: Arc::new(Mutex::new(Vec::new())),
            data_loaded: DataLoaded {
                jobs: false,
                repos: false,
                snapshots: false,
            },
            daemon_running: daemon_running(),
            last_daemon_check: Instant::now(),
            show_help: false,
        }
    }

    fn push_log(&mut self, line: String) {
        self.logs.push(line);
    }

    fn set_status(&mut self, msg: String) {
        self.status_message = Some((msg, Instant::now()));
    }

    fn current_exe() -> Result<String, AppError> {
        std::env::current_exe()
            .map_err(|e| AppError::Other(format!("current_exe: {e}")))?
            .to_str()
            .ok_or_else(|| AppError::Other("current_exe is not valid UTF-8".to_string()))
            .map(String::from)
    }
}

fn daemon_running() -> bool {
    #[cfg(target_os = "linux")]
    {
        use std::fs;
        let entries = match fs::read_dir("/proc") {
            Ok(e) => e,
            Err(_) => return false,
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
                continue;
            };
            if !name.chars().all(|c| c.is_ascii_digit()) {
                continue;
            }
            // Read /proc/<pid>/comm to get the process name.
            let comm = match fs::read_to_string(path.join("comm")) {
                Ok(s) => s.trim().to_string(),
                Err(_) => continue,
            };
            if comm != "restic-manager" {
                continue;
            }
            // Check cmdline for "daemon".
            let cmdline = match fs::read_to_string(path.join("cmdline")) {
                Ok(s) => s.replace('\0', " "),
                Err(_) => continue,
            };
            if cmdline.contains("daemon") {
                return true;
            }
        }
        false
    }
    #[cfg(not(target_os = "linux"))]
    {
        // Daemon detection not implemented for this platform.
        false
    }
}

/// Routes `tracing` output into the TUI's log buffer.
///
/// The global subscriber installed by `cli_run` writes to stderr, which the
/// alternate screen shares with the rendered widgets, so the TUI replaces it
/// with one that writes into the log view instead.
fn install_tracing(logs: LogSink) {
    let writer = LogWriter { sink: logs };
    tracing_subscriber::fmt()
        .with_env_filter(crate::cli::build_env_filter())
        .with_writer(writer)
        .with_ansi(false)
        .try_init()
        .ok();
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

pub fn run() -> Result<(), AppError> {
    // Install the tracing subscriber before loading the config, so that
    // warnings raised while loading (an insecure secrets file, say) land
    // in the log view rather than being lost.
    let logs = LogSink::default();
    install_tracing(logs.clone());

    // Load the configuration before the alternate screen is entered, so a
    // config that cannot be read at all is reported as a normal error
    // rather than corrupting the display. A config that loads but has
    // problems is not fatal: the TUI starts and shows them.
    let config = TuiConfig::load()?;

    let mut terminal = setup_terminal()?;
    let result = event_loop(&mut terminal, config, logs);
    restore_terminal(&mut terminal).ok();
    result
}

fn kill_remaining_children(child_pids: &Arc<Mutex<Vec<u32>>>) {
    let pids: Vec<u32> = child_pids
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .drain(..)
        .collect();
    for pid in pids {
        let _ = std::process::Command::new("kill")
            .arg("-TERM")
            .arg(pid.to_string())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
}

// ---------------------------------------------------------------------------
// Terminal setup / teardown
// ---------------------------------------------------------------------------

fn setup_terminal() -> Result<Terminal<CrosstermBackend<Stdout>>, AppError> {
    crossterm::terminal::enable_raw_mode()
        .map_err(|e| AppError::Other(format!("enable_raw_mode: {e}")))?;
    let mut stdout = io::stdout();
    crossterm::execute!(stdout, crossterm::terminal::EnterAlternateScreen)
        .map_err(|e| AppError::Other(format!("EnterAlternateScreen: {e}")))?;
    let backend = CrosstermBackend::new(stdout);
    Terminal::new(backend).map_err(|e| AppError::Other(format!("Terminal::new: {e}")))
}

fn restore_terminal(terminal: &mut Terminal<CrosstermBackend<Stdout>>) -> io::Result<()> {
    crossterm::terminal::disable_raw_mode()?;
    crossterm::execute!(
        terminal.backend_mut(),
        crossterm::terminal::LeaveAlternateScreen
    )?;
    terminal.show_cursor()?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Main event loop
// ---------------------------------------------------------------------------

fn event_loop(
    terminal: &mut Terminal<CrosstermBackend<Stdout>>,
    config: TuiConfig,
    logs: LogSink,
) -> Result<(), AppError> {
    let app = Arc::new(Mutex::new(App::new(logs, config.problems.clone())));

    // Draw the first frame immediately so the user sees the UI before data loads.
    terminal
        .draw(|f| {
            let app_lock = app.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
            ui(f, &app_lock);
        })
        .map_err(|e| AppError::Other(format!("draw: {e}")))?;

    // Fill the job and repository lists from the config already in memory.
    // Note: config edits while the TUI is running are not picked up until a
    // restart. A reload key can be added in a future wave.
    load_lists(Arc::clone(&app), &config);

    loop {
        terminal
            .draw(|f| {
                let app_lock = app.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
                ui(f, &app_lock);
            })
            .map_err(|e| AppError::Other(format!("draw: {e}")))?;

        {
            let mut a = app.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
            if a.last_daemon_check.elapsed() >= Duration::from_secs(DAEMON_CHECK_SECONDS) {
                a.daemon_running = daemon_running();
                a.last_daemon_check = Instant::now();
            }
        }

        if event::poll(Duration::from_millis(TICK_MS))
            .map_err(|e| AppError::Other(format!("event::poll: {e}")))?
        {
            if let Event::Key(key) =
                event::read().map_err(|e| AppError::Other(format!("event::read: {e}")))?
            {
                if handle_key(key, Arc::clone(&app), &config)? {
                    let child_pids = {
                        let a = app.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
                        Arc::clone(&a.child_pids)
                    };
                    kill_remaining_children(&child_pids);
                    return Ok(());
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Key handling
// ---------------------------------------------------------------------------

/// Returns `Ok(true)` when the user requested quit.
fn handle_key(key: KeyEvent, app: Arc<Mutex<App>>, config: &TuiConfig) -> Result<bool, AppError> {
    {
        let mut a = app.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        if a.show_help {
            if key.code == KeyCode::Char('?') || key.code == KeyCode::Esc {
                a.show_help = false;
            }
            // Any other key also dismisses the help overlay.
            return Ok(false);
        }
    }

    match key.code {
        KeyCode::Char('q') | KeyCode::Esc => return Ok(true),
        KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => return Ok(true),
        KeyCode::Char('?') => {
            let mut a = app.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
            a.show_help = true;
        }
        KeyCode::Down => {
            let mut a = app.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
            move_selection(&mut a.sidebar_state, 1);
        }
        KeyCode::Up => {
            let mut a = app.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
            move_selection(&mut a.sidebar_state, -1);
        }
        KeyCode::Char('n') | KeyCode::Char('j') => {
            let mut a = app.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
            advance_list_index(&mut a);
        }
        KeyCode::Char('k') => {
            let mut a = app.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
            retreat_list_index(&mut a);
        }
        KeyCode::Enter => {
            let mut a = app.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
            let selected = a.sidebar_state.selected().unwrap_or(0);
            let view = SIDEBAR_ITEMS.get(selected).copied().unwrap_or("Jobs");
            match view {
                "Jobs" if !a.jobs.is_empty() && a.job_list_index < a.jobs.len() => {
                    a.selected_job = Some(a.jobs[a.job_list_index].clone());
                    a.job_details = None;
                    let name = a.selected_job.clone().unwrap();
                    drop(a);
                    load_job_details(Arc::clone(&app), config, &name);
                }
                "Repositories" if !a.repos.is_empty() && a.repo_list_index < a.repos.len() => {
                    a.selected_repo = Some(a.repos[a.repo_list_index].clone());
                    a.repo_details = None;
                    let name = a.selected_repo.clone().unwrap();
                    drop(a);
                    load_repo_details(Arc::clone(&app), config, &name);
                }
                _ => {}
            }
        }
        KeyCode::Char('r') => {
            let mut a = app.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
            if let Some(job) = a.selected_job.clone() {
                if command_running(&a) {
                    a.set_status("A command is already running".to_string());
                } else {
                    drop(a);
                    run_command(
                        Arc::clone(&app),
                        format!("run: {job}"),
                        vec!["run".to_string(), job],
                    );
                }
            } else {
                a.set_status("Select a job first".to_string());
            }
        }
        KeyCode::Char('R') => {
            let mut a = app.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
            a.set_status(
                "Restore: select snapshot and target — full overlay in Wave 4 \
                 (use CLI: restic-manager restore <job> --target <dir>)"
                    .to_string(),
            );
        }
        KeyCode::Char('l') => {
            let job_name = {
                let a = app.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
                a.selected_job.clone()
            };
            if let Some(job) = job_name {
                {
                    let mut a = app.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
                    a.set_status(format!("Loading snapshots for {job}..."));
                    a.data_loaded.snapshots = false;
                    a.snapshots.clear();
                }
                let a = Arc::clone(&app);
                let config = config.clone();
                std::thread::spawn(move || load_snapshots_async(a, config, job));
            } else {
                let mut a = app.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
                a.set_status("Select a job first".to_string());
            }
        }
        KeyCode::Char('p') | KeyCode::Char('P') => {
            let mut a = app.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
            if let Some(job) = a.selected_job.clone() {
                if command_running(&a) {
                    a.set_status("A command is already running".to_string());
                } else {
                    drop(a);
                    run_command(
                        Arc::clone(&app),
                        format!("prune: {job}"),
                        vec!["prune".to_string(), job],
                    );
                }
            } else {
                a.set_status("Select a job first".to_string());
            }
        }
        KeyCode::Char('c') => {
            let mut a = app.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
            if let Some(job) = a.selected_job.clone() {
                if command_running(&a) {
                    a.set_status("A command is already running".to_string());
                } else {
                    drop(a);
                    run_command(
                        Arc::clone(&app),
                        format!("check: {job}"),
                        vec!["check".to_string(), job],
                    );
                }
            } else {
                a.set_status("Select a job first".to_string());
            }
        }
        KeyCode::Char('L') => {
            let mut a = app.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
            a.sidebar_state.select(Some(2));
        }
        _ => {}
    }
    Ok(false)
}

fn move_selection(state: &mut ListState, delta: i32) {
    let len = SIDEBAR_ITEMS.len() as i32;
    let cur = state.selected().unwrap_or(0) as i32;
    let next = (cur + delta).rem_euclid(len) as usize;
    state.select(Some(next));
}

fn advance_list_index(app: &mut App) {
    let selected = app.sidebar_state.selected().unwrap_or(0);
    let view = SIDEBAR_ITEMS.get(selected).copied().unwrap_or("Jobs");
    match view {
        "Jobs" if !app.jobs.is_empty() => {
            app.job_list_index = (app.job_list_index + 1) % app.jobs.len();
        }
        "Repositories" if !app.repos.is_empty() => {
            app.repo_list_index = (app.repo_list_index + 1) % app.repos.len();
        }
        _ => {}
    }
}

fn retreat_list_index(app: &mut App) {
    let selected = app.sidebar_state.selected().unwrap_or(0);
    let view = SIDEBAR_ITEMS.get(selected).copied().unwrap_or("Jobs");
    match view {
        "Jobs" if !app.jobs.is_empty() => {
            let len = app.jobs.len();
            app.job_list_index = (app.job_list_index + len - 1) % len;
        }
        "Repositories" if !app.repos.is_empty() => {
            let len = app.repos.len();
            app.repo_list_index = (app.repo_list_index + len - 1) % len;
        }
        _ => {}
    }
}

fn command_running(app: &App) -> bool {
    app.running_label
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .is_some()
}

// ---------------------------------------------------------------------------
// Data loading (background threads)
// ---------------------------------------------------------------------------

/// The configuration the TUI works from.
///
/// Loaded leniently: a repository whose secret is missing is reported as a
/// problem rather than aborting the load, so one broken repository does not
/// hide every other job from the dashboard.
#[derive(Clone)]
struct TuiConfig {
    resolved: ResolvedConfig,
    problems: Vec<String>,
}

impl TuiConfig {
    fn load() -> Result<Self, AppError> {
        let (resolved, problems) = ResolvedConfig::load_partial()?;
        Ok(Self { resolved, problems })
    }

    /// Job names, from the loaded config.
    fn job_names(&self) -> Vec<String> {
        self.resolved
            .config
            .list_jobs()
            .into_iter()
            .map(String::from)
            .collect()
    }

    /// Repository names, from the loaded config.
    fn repo_names(&self) -> Vec<String> {
        self.resolved
            .config
            .list_repositories()
            .into_iter()
            .map(String::from)
            .collect()
    }
}

/// Fills the job and repository lists from the in-memory config.
///
/// No subprocess is spawned and no JSON is parsed: the data is already in
/// the same process.
fn load_lists(app: Arc<Mutex<App>>, config: &TuiConfig) {
    let mut a = app.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    a.jobs = config.job_names();
    a.repos = config.repo_names();
    a.data_loaded.jobs = true;
    a.data_loaded.repos = true;
    for problem in &config.problems {
        a.push_log(format!("config problem: {problem}"));
    }
}

/// Loads the details of one job from the in-memory config.
fn load_job_details(app: Arc<Mutex<App>>, config: &TuiConfig, job_name: &str) {
    let job = config.resolved.config.get_job(job_name).cloned();
    let mut a = app.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    match job {
        Some(job) => {
            if a.selected_job.as_deref() == Some(job_name) {
                a.job_details = Some(job);
            }
        }
        None => {
            a.push_log(format!("job '{job_name}' not found in config"));
        }
    }
}

/// Loads the details of one repository from the in-memory config.
fn load_repo_details(app: Arc<Mutex<App>>, config: &TuiConfig, repo_name: &str) {
    let repo = config.resolved.config.get_repository(repo_name).cloned();
    let mut a = app.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    match repo {
        Some(repo) => {
            if a.selected_repo.as_deref() == Some(repo_name) {
                a.repo_details = Some(repo);
            }
        }
        None => {
            a.push_log(format!("repository '{repo_name}' not found in config"));
        }
    }
}

/// Loads the snapshots for one job by calling `restic` directly.
///
/// This is the one data load that genuinely needs a subprocess: it talks to
/// the repository. It runs on a background thread so the UI stays
/// responsive, and it reports failures into the log view rather than
/// writing to the terminal.
fn load_snapshots_async(app: Arc<Mutex<App>>, config: TuiConfig, job_name: String) {
    let result = SnapshotManager::list(&config.resolved, &job_name);
    let mut a = app.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    match result {
        Ok(list) => {
            a.snapshots = list.snapshots;
            a.data_loaded.snapshots = true;
        }
        Err(e) => {
            a.data_loaded.snapshots = true;
            a.push_log(format!("error loading snapshots for {job_name}: {e}"));
        }
    }
}

// ---------------------------------------------------------------------------
// Background command execution
// ---------------------------------------------------------------------------

/// Spawn a long-running `restic-manager <args>` command on a background thread
/// and stream its stdout/stderr into the shared log buffer. The UI stays
/// responsive because `child.wait()` happens off the main thread.
///
/// The child's PID is recorded in `app.child_pids` so the TUI can send SIGTERM
/// to any remaining children on exit.
fn run_command(app: Arc<Mutex<App>>, label: String, args: Vec<String>) {
    let (logs, child_pids) = {
        let mut a = app.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        *a.running_label
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(label.clone());
        a.push_log(format!("--- starting: {label} ---"));
        // Return independent clones so the worker thread can clear state without
        // holding the main app mutex.
        (Arc::clone(&a.running_label), Arc::clone(&a.child_pids))
    };

    std::thread::spawn(move || {
        let exe = match App::current_exe() {
            Ok(exe) => exe,
            Err(e) => {
                push_log_and_clear(&app, &logs, format!("error spawning command: {e}"));
                return;
            }
        };

        let mut child = match Command::new(&exe)
            .args(&args)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
        {
            Ok(child) => child,
            Err(e) => {
                push_log_and_clear(&app, &logs, format!("error spawning command: {e}"));
                return;
            }
        };

        let pid = child.id();
        child_pids
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .push(pid);

        let stdout = child.stdout.take();
        let stderr = child.stderr.take();
        let app_stdout = Arc::clone(&app);
        let app_stderr = Arc::clone(&app);

        if let Some(stdout) = stdout {
            std::thread::spawn(move || {
                let reader = BufReader::new(stdout);
                for line in reader.lines().map_while(Result::ok) {
                    let mut a = app_stdout
                        .lock()
                        .unwrap_or_else(|poisoned| poisoned.into_inner());
                    a.push_log(line);
                }
            });
        }

        if let Some(stderr) = stderr {
            std::thread::spawn(move || {
                let reader = BufReader::new(stderr);
                for line in reader.lines().map_while(Result::ok) {
                    let mut a = app_stderr
                        .lock()
                        .unwrap_or_else(|poisoned| poisoned.into_inner());
                    a.push_log(line);
                }
            });
        }

        let exit = child.wait().map(|s| s.code());
        child_pids
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .retain(|p| *p != pid);

        let (finish, status) = match exit {
            Ok(Some(code)) => {
                let status = if code == 0 {
                    None
                } else {
                    Some(format!("error: {label} failed (exit {code})"))
                };
                (
                    format!("--- command finished: exit code {code} ---"),
                    status,
                )
            }
            Ok(None) => ("--- command finished: no exit code ---".to_string(), None),
            Err(ref e) => (
                format!("--- command finished: wait error {e} ---"),
                Some(format!("error: {label} failed (wait error)")),
            ),
        };

        {
            let mut a = app.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
            if let Some(msg) = status {
                a.set_status(msg);
            }
            a.push_log(finish);
        }
        *logs.lock().unwrap_or_else(|poisoned| poisoned.into_inner()) = None;
    });
}

fn push_log_and_clear(app: &Arc<Mutex<App>>, label: &Arc<Mutex<Option<String>>>, line: String) {
    {
        let mut a = app.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        a.push_log(line);
    }
    *label
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner()) = None;
}

// ---------------------------------------------------------------------------
// UI rendering
// ---------------------------------------------------------------------------

fn ui(f: &mut ratatui::Frame, app: &App) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3), // header
            Constraint::Min(1),    // body
            Constraint::Length(1), // status bar
        ])
        .split(f.area());

    // Header
    let daemon_dot = if app.daemon_running {
        Span::styled("●", Style::default().fg(Color::Green))
    } else {
        Span::styled("○", Style::default().fg(Color::Red))
    };
    let header_text = Text::from(Line::from(vec![
        Span::raw("restic-manager — tui  daemon: "),
        daemon_dot,
    ]));
    let header = Paragraph::new(header_text)
        .style(
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        )
        .block(Block::default().borders(Borders::BOTTOM));
    f.render_widget(header, chunks[0]);

    // Body: sidebar + detail
    let body = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Length(20), Constraint::Min(1)])
        .split(chunks[1]);

    let items: Vec<ListItem> = SIDEBAR_ITEMS
        .iter()
        .map(|s| ListItem::new(Line::from(*s)))
        .collect();
    let list = List::new(items)
        .block(Block::default().borders(Borders::RIGHT).title("Views"))
        .highlight_style(
            Style::default()
                .bg(Color::DarkGray)
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol("> ");
    f.render_stateful_widget(list, body[0], &mut app.sidebar_state.clone());

    let selected = app.sidebar_state.selected().unwrap_or(0);
    let selected_view = SIDEBAR_ITEMS.get(selected).copied().unwrap_or("Jobs");
    let detail = match selected_view {
        "Jobs" => render_jobs(app),
        "Repositories" => render_repos(app),
        _ => render_logs(app, body[1].height as usize),
    };
    f.render_widget(detail, body[1]);

    // Status bar
    let status_text = build_status_text(app);
    let status =
        Paragraph::new(status_text).style(Style::default().bg(Color::Reset).fg(Color::Gray));
    f.render_widget(status, chunks[2]);

    if app.show_help {
        render_help(f);
    }
}

fn render_help(f: &mut ratatui::Frame) {
    let help_text = "\
Keybindings:\n\
\n\
↑/↓ or j/k — select sidebar item\n\
n / k — move selection inside the current list (j also down)\n\
Enter — select highlighted job/repository\n\
r — run selected job\n\
R — restore (full overlay coming soon)\n\
l — list snapshots for selected job\n\
p — prune selected job\n\
c — check selected job\n\
L — switch to Logs view\n\
? — toggle this help\n\
q / Esc / Ctrl-C — quit";

    let block = Block::default()
        .borders(Borders::ALL)
        .title("Help")
        .title_alignment(Alignment::Center)
        .border_style(Style::default().fg(Color::Cyan));
    let paragraph = Paragraph::new(help_text)
        .block(block)
        .alignment(Alignment::Center)
        .wrap(ratatui::widgets::Wrap { trim: true });

    let area = centered_rect(60, 60, f.area());
    f.render_widget(Clear, area);
    f.render_widget(paragraph, area);
}

fn centered_rect(percent_x: u16, percent_y: u16, r: Rect) -> Rect {
    let popup_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - percent_y) / 2),
            Constraint::Percentage(percent_y),
            Constraint::Percentage((100 - percent_y) / 2),
        ])
        .split(r);

    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(popup_layout[1])[1]
}

fn render_jobs(app: &App) -> Paragraph<'_> {
    let mut lines = Vec::new();

    if !app.problems.is_empty() {
        lines.push(Line::from(format!(
            "Config problems ({}):",
            app.problems.len()
        )));
        for problem in &app.problems {
            lines.push(Line::from(format!("  {problem}")));
        }
        lines.push(Line::from(""));
    }

    if app.data_loaded.jobs {
        if app.jobs.is_empty() {
            lines.push(Line::from("No jobs configured."));
        } else {
            lines.push(Line::from("Jobs:"));
            for (idx, job) in app.jobs.iter().enumerate() {
                let selected_marker = if app.selected_job.as_deref() == Some(job) {
                    "* "
                } else {
                    "  "
                };
                let highlight_marker = if idx == app.job_list_index {
                    "> "
                } else {
                    "  "
                };
                lines.push(Line::from(format!(
                    "{highlight_marker}{selected_marker}{job}"
                )));
            }
        }
    } else {
        lines.push(Line::from("Loading..."));
    }

    lines.push(Line::from(""));

    if let Some(job) = &app.selected_job {
        lines.push(Line::from(format!("Selected job: {job}")));
        if let Some(details) = &app.job_details {
            lines.push(Line::from(format!("  repository: {}", details.repository)));
            let paths: Vec<String> = details
                .paths
                .iter()
                .map(|p| p.to_string_lossy().to_string())
                .collect();
            lines.push(Line::from(format!("  paths: {}", paths.join(", "))));
            if let Some(schedule) = &details.schedule {
                lines.push(Line::from(format!("  schedule: {}", schedule)));
                lines.push(Line::from(
                    "  Next run: (computed from schedule — not yet implemented)".to_string(),
                ));
            } else {
                lines.push(Line::from("  schedule: (none)"));
                lines.push(Line::from("  Next run: (no schedule)"));
            }
            if let Some(retention) = &details.retention {
                lines.push(Line::from("  retention:"));
                if let Some(v) = retention.keep_last {
                    lines.push(Line::from(format!("    keep_last: {}", v)));
                }
                if let Some(v) = retention.keep_hourly {
                    lines.push(Line::from(format!("    keep_hourly: {}", v)));
                }
                if let Some(v) = retention.keep_daily {
                    lines.push(Line::from(format!("    keep_daily: {}", v)));
                }
                if let Some(v) = retention.keep_weekly {
                    lines.push(Line::from(format!("    keep_weekly: {}", v)));
                }
                if let Some(v) = retention.keep_monthly {
                    lines.push(Line::from(format!("    keep_monthly: {}", v)));
                }
                if let Some(v) = retention.keep_yearly {
                    lines.push(Line::from(format!("    keep_yearly: {}", v)));
                }
            } else {
                lines.push(Line::from("  retention: (none)"));
            }
            lines.push(Line::from(format!(
                "  notifications: on_failure={} on_success={}",
                details.notifications.on_failure, details.notifications.on_success
            )));
            lines.push(Line::from(format!(
                "  hooks: pre_backup={} post_backup={}",
                details.pre_backup.len(),
                details.post_backup.len()
            )));
            lines.push(Line::from(
                "  Last run: (not tracked — would require run-history feature)".to_string(),
            ));
        } else {
            lines.push(Line::from("Loading details..."));
        }

        if app.data_loaded.snapshots {
            lines.push(Line::from(""));
            lines.push(Line::from("Snapshots:"));
            if app.snapshots.is_empty() {
                lines.push(Line::from("  (none)"));
            } else {
                for snap in &app.snapshots {
                    lines.push(Line::from(format!("  {}  {}", snap.short_id, snap.time)));
                }
            }
        } else {
            lines.push(Line::from("Loading snapshots..."));
        }
    } else {
        lines.push(Line::from(
            "Select a job from the list (n/k to move, Enter to select)",
        ));
    }

    Paragraph::new(lines).block(Block::default().borders(Borders::ALL).title("Detail"))
}

fn render_repos(app: &App) -> Paragraph<'_> {
    let mut lines = Vec::new();
    if app.data_loaded.repos {
        if app.repos.is_empty() {
            lines.push(Line::from("No repositories configured."));
        } else {
            lines.push(Line::from("Repositories:"));
            for (idx, repo) in app.repos.iter().enumerate() {
                let selected_marker = if app.selected_repo.as_deref() == Some(repo) {
                    "* "
                } else {
                    "  "
                };
                let highlight_marker = if idx == app.repo_list_index {
                    "> "
                } else {
                    "  "
                };
                lines.push(Line::from(format!(
                    "{highlight_marker}{selected_marker}{repo}"
                )));
            }
        }
    } else {
        lines.push(Line::from("Loading..."));
    }

    lines.push(Line::from(""));

    if let Some(repo) = &app.selected_repo {
        lines.push(Line::from(format!("Selected repository: {repo}")));
        if let Some(details) = &app.repo_details {
            lines.push(Line::from(format!("  repo: {}", details.repo)));
            lines.push(Line::from("  password_key: (configured)"));
            if let Some(log_cli_output) = &details.log_cli_output {
                lines.push(Line::from(format!(
                    "  log_cli_output: {}",
                    log_cli_output.display()
                )));
            }
        } else {
            lines.push(Line::from("Loading details..."));
        }
    } else {
        lines.push(Line::from(
            "Select a repository from the list (n/k to move, Enter to select)",
        ));
    }

    Paragraph::new(lines).block(Block::default().borders(Borders::ALL).title("Detail"))
}

fn render_logs(app: &App, height: usize) -> Paragraph<'static> {
    let n = height.saturating_sub(2).max(1);
    let lines: Vec<Line<'static>> = app.logs.tail(n).into_iter().map(Line::from).collect();
    Paragraph::new(lines).block(Block::default().borders(Borders::ALL).title("Logs"))
}

fn build_status_text(app: &App) -> String {
    if let Some((msg, at)) = &app.status_message {
        if at.elapsed() < Duration::from_secs(STATUS_MESSAGE_SECONDS) {
            return msg.clone();
        }
    }

    if !app.problems.is_empty() {
        return format!(
            "config: {} problem(s) - see Logs  ?:help q:quit",
            app.problems.len()
        );
    }

    if let Some(label) = app
        .running_label
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .as_ref()
    {
        return format!("running: {label}  ?:help  q:quit");
    }

    "?:help n/k list Enter select r:run R:restore l:list p:prune c:check L:logs q:quit".to_string()
}
