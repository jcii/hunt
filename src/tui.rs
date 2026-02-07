use anyhow::Result;
use crossterm::{
    event::{self, Event, KeyCode, KeyEventKind},
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
    ExecutableCommand,
};
use ratatui::{
    prelude::*,
    widgets::{Block, Borders, List, ListItem, ListState, Paragraph, Wrap},
};
use std::io::stdout;

use crate::db::Database;
use crate::models::{Job, JobKeyword, JobKeywordProfile};

struct AppState {
    jobs: Vec<Job>,
    selected: usize,
    scroll_offset: u16,
    keywords: Vec<JobKeyword>,
    profile: Option<JobKeywordProfile>,
    keyword_model: Option<String>,
}

impl AppState {
    fn new(jobs: Vec<Job>) -> Self {
        Self {
            jobs,
            selected: 0,
            scroll_offset: 0,
            keywords: Vec::new(),
            profile: None,
            keyword_model: None,
        }
    }

    fn current_job(&self) -> Option<&Job> {
        self.jobs.get(self.selected)
    }

    fn load_keywords(&mut self, db: &Database) {
        let Some(job) = self.current_job() else { return };
        let job_id = job.id;

        self.keyword_model = db.get_latest_keyword_model(job_id).ok().flatten();
        if let Some(model) = &self.keyword_model {
            self.keywords = db.get_job_keywords(job_id, Some(model)).unwrap_or_default();
            self.profile = db.get_keyword_profile(job_id).ok().flatten();
        } else {
            self.keywords.clear();
            self.profile = None;
        }
    }

    fn next(&mut self) {
        if !self.jobs.is_empty() && self.selected < self.jobs.len() - 1 {
            self.selected += 1;
            self.scroll_offset = 0;
        }
    }

    fn prev(&mut self) {
        if self.selected > 0 {
            self.selected -= 1;
            self.scroll_offset = 0;
        }
    }

    fn scroll_down(&mut self) {
        self.scroll_offset = self.scroll_offset.saturating_add(3);
    }

    fn scroll_up(&mut self) {
        self.scroll_offset = self.scroll_offset.saturating_sub(3);
    }
}

pub fn run_browse(db: &Database, status: Option<&str>, employer: Option<&str>) -> Result<()> {
    let jobs = db.list_jobs(status, employer)?;
    if jobs.is_empty() {
        println!("No jobs found.");
        return Ok(());
    }

    let mut state = AppState::new(jobs);
    state.load_keywords(db);

    // Setup terminal
    enable_raw_mode()?;
    stdout().execute(EnterAlternateScreen)?;
    let mut terminal = Terminal::new(CrosstermBackend::new(stdout()))?;

    let result = run_loop(&mut terminal, &mut state, db);

    // Restore terminal
    disable_raw_mode()?;
    stdout().execute(LeaveAlternateScreen)?;

    result
}

fn run_loop(
    terminal: &mut Terminal<CrosstermBackend<std::io::Stdout>>,
    state: &mut AppState,
    db: &Database,
) -> Result<()> {
    let mut list_state = ListState::default();
    list_state.select(Some(0));

    loop {
        terminal.draw(|frame| draw(frame, state, &mut list_state))?;

        if let Event::Key(key) = event::read()? {
            if key.kind != KeyEventKind::Press {
                continue;
            }
            let prev_selected = state.selected;
            match key.code {
                KeyCode::Char('q') | KeyCode::Esc => break,
                KeyCode::Down | KeyCode::Char('j') => state.next(),
                KeyCode::Up | KeyCode::Char('k') => state.prev(),
                KeyCode::Char('J') | KeyCode::PageDown => state.scroll_down(),
                KeyCode::Char('K') | KeyCode::PageUp => state.scroll_up(),
                KeyCode::Char('n') => {
                    if let Some(job) = state.current_job() {
                        let _ = db.update_job_status(job.id, "new");
                        if let Some(j) = state.jobs.get_mut(state.selected) {
                            j.status = "new".to_string();
                        }
                    }
                }
                KeyCode::Char('r') => {
                    if let Some(job) = state.current_job() {
                        let _ = db.update_job_status(job.id, "reviewing");
                        if let Some(j) = state.jobs.get_mut(state.selected) {
                            j.status = "reviewing".to_string();
                        }
                    }
                }
                KeyCode::Char('a') => {
                    if let Some(job) = state.current_job() {
                        let _ = db.update_job_status(job.id, "applied");
                        if let Some(j) = state.jobs.get_mut(state.selected) {
                            j.status = "applied".to_string();
                        }
                    }
                }
                KeyCode::Char('x') => {
                    if let Some(job) = state.current_job() {
                        let _ = db.update_job_status(job.id, "rejected");
                        if let Some(j) = state.jobs.get_mut(state.selected) {
                            j.status = "rejected".to_string();
                        }
                    }
                }
                KeyCode::Char('c') => {
                    if let Some(job) = state.current_job() {
                        let _ = db.update_job_status(job.id, "closed");
                        if let Some(j) = state.jobs.get_mut(state.selected) {
                            j.status = "closed".to_string();
                        }
                    }
                }
                _ => {}
            }
            if state.selected != prev_selected {
                list_state.select(Some(state.selected));
                state.load_keywords(db);
            }
        }
    }
    Ok(())
}

fn draw(frame: &mut Frame, state: &AppState, list_state: &mut ListState) {
    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(35),
            Constraint::Percentage(65),
        ])
        .split(frame.area());

    // Left panel: job list
    let items: Vec<ListItem> = state
        .jobs
        .iter()
        .map(|job| {
            let status_icon = match job.status.as_str() {
                "new" => " ",
                "reviewing" => "*",
                "applied" => "+",
                "rejected" => "x",
                "closed" => "-",
                _ => "?",
            };
            let employer = job.employer_name.as_deref().unwrap_or("?");
            let title = if job.title.len() > 35 {
                format!("{}...", &job.title[..32])
            } else {
                job.title.clone()
            };
            ListItem::new(format!("{} #{:<4} {} | {}", status_icon, job.id, title, employer))
        })
        .collect();

    let list = List::new(items)
        .block(Block::default().borders(Borders::ALL).title(format!(
            " Jobs ({}) ", state.jobs.len()
        )))
        .highlight_style(Style::default().bg(Color::DarkGray).add_modifier(Modifier::BOLD))
        .highlight_symbol("> ");

    frame.render_stateful_widget(list, chunks[0], list_state);

    // Right panel: job detail
    let detail = build_detail(state);
    let detail_widget = Paragraph::new(detail)
        .block(Block::default().borders(Borders::ALL).title(" Detail "))
        .wrap(Wrap { trim: false })
        .scroll((state.scroll_offset, 0));

    frame.render_widget(detail_widget, chunks[1]);

    // Footer help
    let help_area = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(0), Constraint::Length(1)])
        .split(frame.area());

    let help = Paragraph::new(
        " j/k:navigate  J/K:scroll  n:new r:reviewing a:applied x:reject c:close  q:quit"
    )
    .style(Style::default().fg(Color::DarkGray));
    frame.render_widget(help, help_area[1]);
}

fn build_detail<'a>(state: &'a AppState) -> Text<'a> {
    let Some(job) = state.current_job() else {
        return Text::raw("No job selected");
    };

    let mut lines: Vec<Line> = Vec::new();

    // Header
    lines.push(Line::from(Span::styled(
        &job.title,
        Style::default().add_modifier(Modifier::BOLD),
    )));

    if let Some(employer) = &job.employer_name {
        lines.push(Line::from(format!("at {}", employer)));
    }

    let status_style = match job.status.as_str() {
        "new" => Style::default().fg(Color::Green),
        "reviewing" => Style::default().fg(Color::Yellow),
        "applied" => Style::default().fg(Color::Cyan),
        "rejected" => Style::default().fg(Color::Red),
        "closed" => Style::default().fg(Color::DarkGray),
        _ => Style::default(),
    };
    lines.push(Line::from(Span::styled(
        format!("Status: {}", job.status),
        status_style,
    )));

    if let Some(url) = &job.url {
        lines.push(Line::from(format!("URL: {}", url)));
    }

    match (job.pay_min, job.pay_max) {
        (Some(min), Some(max)) => lines.push(Line::from(format!("Pay: ${} - ${}", min, max))),
        (Some(min), None) => lines.push(Line::from(format!("Pay: ${}+", min))),
        (None, Some(max)) => lines.push(Line::from(format!("Pay: up to ${}", max))),
        (None, None) => {}
    }

    lines.push(Line::from(""));

    // Keywords
    if !state.keywords.is_empty() {
        let model = state.keyword_model.as_deref().unwrap_or("?");
        lines.push(Line::from(Span::styled(
            format!("Keywords ({})", model),
            Style::default().add_modifier(Modifier::BOLD),
        )));
        lines.push(Line::from(
            Span::styled("*** required  ** important  * nice-to-have", Style::default().fg(Color::DarkGray))
        ));
        lines.push(Line::from(""));

        let domains = [
            ("tech", "TECH"),
            ("discipline", "DISCIPLINE"),
            ("cloud", "CLOUD"),
            ("soft_skill", "SOFT SKILLS"),
        ];

        for (domain_key, domain_label) in &domains {
            let domain_kws: Vec<&JobKeyword> = state
                .keywords
                .iter()
                .filter(|k| k.domain == *domain_key)
                .collect();

            if domain_kws.is_empty() {
                continue;
            }

            lines.push(Line::from(Span::styled(
                format!("  {}", domain_label),
                Style::default().fg(Color::Cyan),
            )));

            for weight in (1..=3).rev() {
                let at_weight: Vec<&str> = domain_kws
                    .iter()
                    .filter(|k| k.weight == weight)
                    .map(|k| k.keyword.as_str())
                    .collect();

                if at_weight.is_empty() {
                    continue;
                }

                let stars = "*".repeat(weight as usize);
                let pad = " ".repeat(3 - weight as usize);
                lines.push(Line::from(format!("    {}{} {}", pad, stars, at_weight.join(", "))));
            }
        }

        lines.push(Line::from(""));

        // Profile
        if let Some(profile) = &state.profile {
            lines.push(Line::from(Span::styled(
                "PROFILE",
                Style::default().add_modifier(Modifier::BOLD),
            )));
            for line in textwrap::fill(&profile.profile, 70).lines() {
                lines.push(Line::from(format!("  {}", line)));
            }
            lines.push(Line::from(""));
        }
    } else if job.raw_text.is_some() {
        lines.push(Line::from(Span::styled(
            "(No keywords — run: hunt keywords {})",
            Style::default().fg(Color::DarkGray),
        )));
        lines.push(Line::from(""));

        // Show raw text if no keywords
        if let Some(text) = &job.raw_text {
            lines.push(Line::from(Span::styled(
                "Raw Description",
                Style::default().add_modifier(Modifier::BOLD),
            )));
            for line in text.lines() {
                lines.push(Line::from(line.to_string()));
            }
        }
    } else {
        lines.push(Line::from(Span::styled(
            "(No description fetched)",
            Style::default().fg(Color::DarkGray),
        )));
    }

    Text::from(lines)
}
