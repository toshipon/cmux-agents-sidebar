use std::collections::HashSet;

use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Clear, Paragraph},
};

use crate::model::{Task, format_age};
use crate::state::AgentLane;

pub enum ViewStatus<'a> {
    Ready,
    Reconnecting { message: &'a str },
}

#[derive(Clone, Copy)]
pub enum TaskPart {
    Title,
    Meta,
    Reason,
}

pub enum FlatRow<'a> {
    Group {
        lane: AgentLane,
        count: usize,
        collapsed: bool,
    },
    Task {
        task: &'a Task,
        part: TaskPart,
    },
}

impl<'a> FlatRow<'a> {
    pub fn is_selectable(&self) -> bool {
        match self {
            Self::Group { .. } => true,
            Self::Task { part, .. } => matches!(part, TaskPart::Title),
        }
    }

    pub fn task(&self) -> Option<&'a Task> {
        match self {
            Self::Task { task, .. } => Some(*task),
            Self::Group { .. } => None,
        }
    }

    pub fn lane(&self) -> AgentLane {
        match self {
            Self::Group { lane, .. } => *lane,
            Self::Task { task, .. } => task.lane,
        }
    }
}

pub fn flatten_rows<'a>(
    groups: &'a [(AgentLane, Vec<Task>)],
    collapsed: &HashSet<AgentLane>,
) -> Vec<FlatRow<'a>> {
    let mut rows = Vec::new();
    for (lane, tasks) in groups {
        let collapsed = collapsed.contains(lane);
        rows.push(FlatRow::Group {
            lane: *lane,
            count: tasks.len(),
            collapsed,
        });
        if collapsed {
            continue;
        }
        for task in tasks {
            rows.push(FlatRow::Task {
                task,
                part: TaskPart::Title,
            });
            rows.push(FlatRow::Task {
                task,
                part: TaskPart::Meta,
            });
            rows.push(FlatRow::Task {
                task,
                part: TaskPart::Reason,
            });
        }
    }
    rows
}

pub fn draw(frame: &mut Frame<'_>, rows: &[FlatRow<'_>], selected: usize, status: ViewStatus<'_>) {
    let area = frame.area();
    frame.render_widget(Clear, area);
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Min(0),
            Constraint::Length(1),
        ])
        .split(area);
    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(
            "AGENTS",
            Style::new().add_modifier(Modifier::BOLD | Modifier::DIM),
        ))),
        chunks[0],
    );
    match status {
        ViewStatus::Reconnecting { message } => draw_reconnect(frame, chunks[1], message),
        ViewStatus::Ready if rows.is_empty() => {
            frame.render_widget(Paragraph::new("No workspaces"), chunks[1]);
        }
        ViewStatus::Ready => {
            let height = chunks[1].height as usize;
            let offset = scroll_offset(selected, height, rows.len());
            for (line_idx, row) in rows.iter().skip(offset).take(height).enumerate() {
                let y = chunks[1].y + line_idx as u16;
                let is_selected = offset + line_idx == selected;
                frame.render_widget(
                    Paragraph::new(flat_line(row, chunks[1].width as usize, is_selected)),
                    Rect::new(chunks[1].x, y, chunks[1].width, 1),
                );
            }
        }
    }
    let hint = match status {
        ViewStatus::Ready => "↑↓ enter  tab group  space fold  r refresh  q quit",
        ViewStatus::Reconnecting { .. } => "waiting for CMUX_TUI_SOCKET",
    };
    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(
            hint,
            Style::new().add_modifier(Modifier::DIM),
        ))),
        chunks[2],
    );
}

fn draw_reconnect(frame: &mut Frame<'_>, area: Rect, message: &str) {
    let lines = [
        "Reconnecting to cmux",
        message,
        "Launch from cmux, or set CMUX_TUI_SOCKET.",
    ];
    for (idx, line) in lines.iter().enumerate() {
        if idx >= area.height as usize {
            break;
        }
        frame.render_widget(
            Paragraph::new(middle_truncate(line, area.width as usize)),
            Rect::new(area.x, area.y + idx as u16, area.width, 1),
        );
    }
}

fn flat_line(row: &FlatRow<'_>, width: usize, selected: bool) -> Line<'static> {
    let mut style = if selected && row.is_selectable() {
        Style::new().add_modifier(Modifier::REVERSED)
    } else {
        Style::new()
    };
    let text = match row {
        FlatRow::Group {
            lane,
            count,
            collapsed,
        } => {
            style = style.add_modifier(Modifier::BOLD).fg(group_color(*lane));
            let marker = if *collapsed { "▸" } else { "▾" };
            format!("{marker} {}  {count}", lane.title())
        }
        FlatRow::Task { task, part } => {
            style = style.fg(group_color(task.lane));
            match part {
                TaskPart::Title => {
                    let pr = task
                        .pr_number
                        .map(|number| format!("  #{number}"))
                        .unwrap_or_default();
                    let mark = if task.focused { "*" } else { " " };
                    format!("{mark}{} {}{pr}", task.lane.glyph(), task.workspace_name)
                }
                TaskPart::Meta => {
                    style = style.add_modifier(Modifier::DIM);
                    let extra = format_age(task.age_ms)
                        .map(|age| {
                            if task.subtitle.contains(&age) {
                                String::new()
                            } else {
                                format!(" · {age}")
                            }
                        })
                        .unwrap_or_default();
                    format!("  {}{extra}", task.subtitle)
                }
                TaskPart::Reason => {
                    style = style.add_modifier(Modifier::DIM);
                    format!("  {}", task.detail)
                }
            }
        }
    };
    Line::from(Span::styled(
        pad(&middle_truncate(&text, width), width),
        style,
    ))
}

fn group_color(lane: AgentLane) -> Color {
    match lane {
        AgentLane::NeedsAttention => Color::Yellow,
        AgentLane::Working => Color::Cyan,
        AgentLane::InReview => Color::Magenta,
        AgentLane::Done => Color::Green,
        AgentLane::Idle => Color::Gray,
    }
}

fn pad(text: &str, width: usize) -> String {
    let chars = text.chars().count();
    if chars >= width {
        text.to_string()
    } else {
        format!("{text}{}", " ".repeat(width - chars))
    }
}

fn middle_truncate(input: &str, max_chars: usize) -> String {
    let chars: Vec<char> = input.chars().collect();
    if chars.len() <= max_chars {
        return input.to_string();
    }
    if max_chars <= 3 {
        return ".".repeat(max_chars);
    }
    let keep = max_chars - 3;
    let front = keep.div_ceil(2);
    let back = keep / 2;
    let mut out: String = chars.iter().take(front).collect();
    out.push_str("...");
    out.extend(chars.iter().skip(chars.len() - back));
    out
}

fn scroll_offset(selected: usize, visible_height: usize, total: usize) -> usize {
    if visible_height == 0 || total <= visible_height {
        return 0;
    }
    if selected < visible_height {
        return 0;
    }
    (selected + 1)
        .saturating_sub(visible_height)
        .min(total - visible_height)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn middle_truncates() {
        assert_eq!(middle_truncate("abcdefghi", 7), "ab...hi");
    }
}
