use std::collections::HashSet;
use std::io;
use std::time::{Duration, Instant};

use anyhow::Result;
use crossterm::{
    event::{self, Event, KeyCode, KeyEvent, KeyModifiers},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{Terminal, backend::CrosstermBackend};

use cmux_agents_sidebar::cmux::{self, CmuxSession};
use cmux_agents_sidebar::github::GitHubProbe;
use cmux_agents_sidebar::model::{self, GroupedTasks, Task};
use cmux_agents_sidebar::state::AgentLane;
use cmux_agents_sidebar::ui::{self, FlatRow, ViewStatus};

const REFRESH_EVERY: Duration = Duration::from_secs(1);
const POLL_EVERY: Duration = Duration::from_millis(100);
const INITIAL_RECONNECT_DELAY: Duration = Duration::from_millis(500);
const MAX_RECONNECT_DELAY: Duration = Duration::from_secs(8);

fn main() -> Result<()> {
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let _ = disable_raw_mode();
        let _ = execute!(io::stdout(), LeaveAlternateScreen);
        default_hook(info);
    }));
    let mut terminal = setup_terminal()?;
    let result = run(&mut terminal);
    restore_terminal(&mut terminal)?;
    result
}

fn setup_terminal() -> Result<Terminal<CrosstermBackend<io::Stdout>>> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    Ok(Terminal::new(CrosstermBackend::new(stdout))?)
}

fn restore_terminal(terminal: &mut Terminal<CrosstermBackend<io::Stdout>>) -> Result<()> {
    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;
    Ok(())
}

fn run(terminal: &mut Terminal<CrosstermBackend<io::Stdout>>) -> Result<()> {
    let mut app = App::new();
    app.connect_or_schedule();
    loop {
        let rows = app.rows();
        let status = app.view_status();
        terminal.draw(|frame| ui::draw(frame, &rows, app.selected, status))?;
        if event::poll(POLL_EVERY)?
            && let Event::Key(key) = event::read()?
            && app.handle_key(key)
        {
            break;
        }
        app.tick();
    }
    Ok(())
}

struct App {
    grouped: GroupedTasks,
    collapsed: HashSet<AgentLane>,
    selected: usize,
    client: Option<CmuxSession>,
    github: GitHubProbe,
    status: Status,
    last_refresh: Instant,
    next_reconnect: Instant,
    reconnect_delay: Duration,
}

#[derive(Clone)]
enum Status {
    Ready,
    Reconnecting { message: String },
}

impl App {
    fn new() -> Self {
        Self {
            grouped: GroupedTasks { groups: Vec::new() },
            collapsed: HashSet::new(),
            selected: 0,
            client: None,
            github: GitHubProbe::default(),
            status: Status::Reconnecting {
                message: "connecting".into(),
            },
            last_refresh: Instant::now(),
            next_reconnect: Instant::now(),
            reconnect_delay: INITIAL_RECONNECT_DELAY,
        }
    }

    fn view_status(&self) -> ViewStatus<'_> {
        match &self.status {
            Status::Ready => ViewStatus::Ready,
            Status::Reconnecting { message } => ViewStatus::Reconnecting { message },
        }
    }

    fn rows(&self) -> Vec<FlatRow<'_>> {
        ui::flatten_rows(&self.grouped.groups, &self.collapsed)
    }

    fn handle_key(&mut self, key: KeyEvent) -> bool {
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
            return true;
        }
        match key.code {
            KeyCode::Char('q') if key.modifiers.is_empty() => return true,
            KeyCode::Esc => {}
            KeyCode::Up => self.move_selection(-1),
            KeyCode::Char('p') | KeyCode::Char('k')
                if key.modifiers.contains(KeyModifiers::CONTROL) =>
            {
                self.move_selection(-1)
            }
            KeyCode::Down => self.move_selection(1),
            KeyCode::Char('n') | KeyCode::Char('j')
                if key.modifiers.contains(KeyModifiers::CONTROL) =>
            {
                self.move_selection(1)
            }
            KeyCode::Enter => self.activate_selected(),
            KeyCode::Tab => self.move_group(1),
            KeyCode::BackTab => self.move_group(-1),
            KeyCode::Char(' ') => self.toggle_collapse(),
            KeyCode::Char('r') if key.modifiers.is_empty() => self.refresh(),
            _ => {}
        }
        false
    }

    fn tick(&mut self) {
        let now = Instant::now();
        if self.client.is_none() {
            if now >= self.next_reconnect {
                self.connect_or_schedule();
            }
            return;
        }
        if now.duration_since(self.last_refresh) >= REFRESH_EVERY {
            self.refresh();
        }
    }

    fn selectable_indices(&self) -> Vec<usize> {
        self.rows()
            .iter()
            .enumerate()
            .filter(|(_, row)| row.is_selectable())
            .map(|(idx, _)| idx)
            .collect()
    }

    fn move_selection(&mut self, delta: isize) {
        let indices = self.selectable_indices();
        if indices.is_empty() {
            self.selected = 0;
            return;
        }
        let current = indices
            .iter()
            .position(|idx| *idx >= self.selected)
            .unwrap_or(indices.len() - 1);
        let next = current.saturating_add_signed(delta).min(indices.len() - 1);
        self.selected = indices[next];
    }

    fn move_group(&mut self, delta: isize) {
        let groups: Vec<usize> = self
            .rows()
            .iter()
            .enumerate()
            .filter(|(_, row)| matches!(row, FlatRow::Group { .. }))
            .map(|(idx, _)| idx)
            .collect();
        if groups.is_empty() {
            return;
        }
        let current = groups
            .iter()
            .position(|idx| *idx >= self.selected)
            .unwrap_or(0);
        let next = if delta >= 0 {
            (current + 1) % groups.len()
        } else if current == 0 {
            groups.len() - 1
        } else {
            current - 1
        };
        self.selected = groups[next];
    }

    fn toggle_collapse(&mut self) {
        let Some(lane) = self.rows().get(self.selected).map(FlatRow::lane) else {
            return;
        };
        if !self.collapsed.remove(&lane) {
            self.collapsed.insert(lane);
        }
        self.clamp_selection();
    }

    fn activate_selected(&mut self) {
        let task: Option<Task> = self
            .rows()
            .get(self.selected)
            .and_then(FlatRow::task)
            .cloned();
        let Some(task) = task else {
            self.toggle_collapse();
            return;
        };
        let result = match self.client.as_mut() {
            Some(client) => jump(client, &task),
            None => return,
        };
        match result {
            Ok(()) => self.refresh(),
            Err(err) => self.disconnect(format!("cmux command failed: {err}")),
        }
    }

    fn connect_or_schedule(&mut self) {
        let Some(path) = cmux::socket_from_env() else {
            self.disconnect_with_backoff(
                "CMUX_TUI_SOCKET is not set. Launch this plugin from cmux.".into(),
            );
            return;
        };
        match CmuxSession::connect(&path) {
            Ok(mut client) => match client.snapshot() {
                Ok(snapshot) => {
                    self.client = Some(client);
                    self.status = Status::Ready;
                    self.reconnect_delay = INITIAL_RECONNECT_DELAY;
                    self.apply_snapshot(snapshot);
                }
                Err(err) => self.disconnect_with_backoff(format!("cmux did not respond: {err}")),
            },
            Err(err) => self.disconnect_with_backoff(format!("cannot connect to cmux: {err}")),
        }
    }

    fn refresh(&mut self) {
        let result = match self.client.as_mut() {
            Some(client) => client.snapshot(),
            None => return,
        };
        match result {
            Ok(snapshot) => self.apply_snapshot(snapshot),
            Err(err) => self.disconnect(format!("cmux socket dropped: {err}")),
        }
    }

    fn apply_snapshot(&mut self, snapshot: cmux::SessionSnapshot) {
        let tasks = model::build_tasks(&snapshot, &mut self.github, model::now_ms());
        self.grouped = model::group_tasks(tasks);
        self.last_refresh = Instant::now();
        self.clamp_selection();
    }

    fn clamp_selection(&mut self) {
        let indices = self.selectable_indices();
        if indices.is_empty() {
            self.selected = 0;
            return;
        }
        if !indices.contains(&self.selected) {
            self.selected = indices
                .iter()
                .copied()
                .find(|idx| *idx >= self.selected)
                .unwrap_or(*indices.last().unwrap_or(&0));
        }
    }

    fn disconnect(&mut self, message: String) {
        self.client = None;
        self.disconnect_with_backoff(message);
    }

    fn disconnect_with_backoff(&mut self, message: String) {
        self.status = Status::Reconnecting { message };
        self.next_reconnect = Instant::now() + self.reconnect_delay;
        self.reconnect_delay = (self.reconnect_delay * 2).min(MAX_RECONNECT_DELAY);
    }
}

fn jump(client: &mut CmuxSession, task: &Task) -> anyhow::Result<()> {
    client.focus_workspace(&task.workspace_id)?;
    if let Some(tab_id) = &task.tab_id {
        let _ = client.focus_tab(tab_id);
    }
    Ok(())
}
