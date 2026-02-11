use anyhow::Result;
use crossterm::{
    event::{self, Event, KeyCode, KeyEventKind, KeyModifiers},
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
    ExecutableCommand,
};
use ratatui::{
    prelude::*,
    widgets::{Block, Borders, List, ListItem, ListState, Paragraph, Wrap},
};
use std::io::stdout;

use crate::db::{self, Database};
use crate::models::{FitAnalysis, Job, JobKeyword, JobKeywordProfile};

#[derive(Clone, Copy, PartialEq)]
enum SortField {
    Score,
    Salary,
    Fit,
    Company,
}

impl SortField {
    fn label(self) -> &'static str {
        match self {
            SortField::Score => "Score",
            SortField::Salary => "Salary",
            SortField::Fit => "Fit",
            SortField::Company => "Company",
        }
    }
}

struct AppState {
    jobs: Vec<Job>,
    scores: Vec<f64>,              // ranking score per job (parallel to jobs)
    fit_scores: Vec<Option<f64>>,  // raw fit score per job (parallel to jobs)
    visible: Vec<usize>,           // indices into jobs matching current filter, sorted by score
    selected: usize,               // index into visible
    scroll_offset: u16,
    keywords: Vec<JobKeyword>,
    profile: Option<JobKeywordProfile>,
    keyword_model: Option<String>,
    fit_analysis: Option<FitAnalysis>,
    search_active: bool,
    search_query: String,
    hide_closed: bool,
    sort_field: SortField,
    sort_ascending: bool,
}

impl AppState {
    fn new(jobs: Vec<Job>, db: &Database) -> Self {
        let scores: Vec<f64> = jobs.iter().map(|j| db::calculate_score(j, db)).collect();
        let fit_scores: Vec<Option<f64>> = jobs.iter().map(|j| {
            db.get_best_fit_score(j.id).ok().flatten()
        }).collect();

        let mut s = Self {
            visible: Vec::new(),
            jobs,
            scores,
            fit_scores,
            selected: 0,
            scroll_offset: 0,
            keywords: Vec::new(),
            profile: None,
            keyword_model: None,
            fit_analysis: None,
            search_active: false,
            search_query: String::new(),
            hide_closed: true,
            sort_field: SortField::Score,
            sort_ascending: false,
        };
        s.update_filter();
        s
    }

    fn current_job(&self) -> Option<&Job> {
        self.visible.get(self.selected).and_then(|&i| self.jobs.get(i))
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

        self.fit_analysis = db.get_best_fit_analysis(job_id).ok().flatten();
    }

    fn update_filter(&mut self) {
        let query = self.search_query.to_lowercase();
        self.visible = self.jobs.iter().enumerate()
            .filter(|(_, job)| {
                if self.hide_closed && job.status == "closed" {
                    return false;
                }
                if !query.is_empty() {
                    return job.title.to_lowercase().contains(&query)
                        || job.employer_name.as_deref().unwrap_or("").to_lowercase().contains(&query);
                }
                true
            })
            .map(|(i, _)| i)
            .collect();

        // Sort visible indices by current sort field
        self.visible.sort_by(|&a, &b| {
            let ord = match self.sort_field {
                SortField::Score => {
                    self.scores[a].partial_cmp(&self.scores[b]).unwrap_or(std::cmp::Ordering::Equal)
                }
                SortField::Salary => {
                    let sa = self.jobs[a].pay_max.or(self.jobs[a].pay_min).unwrap_or(0);
                    let sb = self.jobs[b].pay_max.or(self.jobs[b].pay_min).unwrap_or(0);
                    sa.cmp(&sb)
                }
                SortField::Fit => {
                    let fa = self.fit_scores[a].unwrap_or(-1.0);
                    let fb = self.fit_scores[b].unwrap_or(-1.0);
                    fa.partial_cmp(&fb).unwrap_or(std::cmp::Ordering::Equal)
                }
                SortField::Company => {
                    let ca = self.jobs[a].employer_name.as_deref().unwrap_or("").to_lowercase();
                    let cb = self.jobs[b].employer_name.as_deref().unwrap_or("").to_lowercase();
                    ca.cmp(&cb)
                }
            };
            if self.sort_ascending { ord } else { ord.reverse() }
        });

        if self.visible.is_empty() {
            self.selected = 0;
        } else {
            self.selected = self.selected.min(self.visible.len() - 1);
        }
        self.scroll_offset = 0;
    }

    fn next(&mut self) {
        if !self.visible.is_empty() && self.selected < self.visible.len() - 1 {
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

    fn page_down(&mut self, page_size: usize) {
        if self.visible.is_empty() { return; }
        self.selected = (self.selected + page_size).min(self.visible.len() - 1);
        self.scroll_offset = 0;
    }

    fn page_up(&mut self, page_size: usize) {
        self.selected = self.selected.saturating_sub(page_size);
        self.scroll_offset = 0;
    }

    fn scroll_down(&mut self) {
        self.scroll_offset = self.scroll_offset.saturating_add(3);
    }

    fn scroll_up(&mut self) {
        self.scroll_offset = self.scroll_offset.saturating_sub(3);
    }

    fn set_sort(&mut self, field: SortField) {
        if self.sort_field == field {
            self.sort_ascending = !self.sort_ascending;
        } else {
            self.sort_field = field;
            // Company defaults ascending (A-Z), others default descending (highest first)
            self.sort_ascending = field == SortField::Company;
        }
        self.update_filter();
    }

    fn update_current_job_status(&mut self, db: &Database, status: &str) {
        if let Some(&idx) = self.visible.get(self.selected) {
            let job_id = self.jobs[idx].id;
            let _ = db.update_job_status(job_id, status);
            self.jobs[idx].status = status.to_string();
            // Recompute score for this job
            self.scores[idx] = db::calculate_score(&self.jobs[idx], db);
        }
    }
}

pub fn run_browse(db: &Database, status: Option<&str>, employer: Option<&str>) -> Result<()> {
    let jobs = db.list_jobs(status, employer)?;
    if jobs.is_empty() {
        println!("No jobs found.");
        return Ok(());
    }

    let mut state = AppState::new(jobs, db);
    state.load_keywords(db);

    enable_raw_mode()?;
    stdout().execute(EnterAlternateScreen)?;
    let mut terminal = Terminal::new(CrosstermBackend::new(stdout()))?;

    let result = run_loop(&mut terminal, &mut state, db);

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

            // Search input mode
            if state.search_active {
                match key.code {
                    KeyCode::Esc => {
                        state.search_active = false;
                        state.search_query.clear();
                        state.update_filter();
                        list_state.select(Some(state.selected));
                        state.load_keywords(db);
                    }
                    KeyCode::Enter => {
                        state.search_active = false;
                        if !state.visible.is_empty() {
                            state.load_keywords(db);
                        }
                    }
                    KeyCode::Backspace => {
                        state.search_query.pop();
                        state.update_filter();
                        list_state.select(Some(state.selected));
                        state.load_keywords(db);
                    }
                    KeyCode::Char(c) => {
                        state.search_query.push(c);
                        state.update_filter();
                        list_state.select(Some(state.selected));
                        state.load_keywords(db);
                    }
                    _ => {}
                }
                continue;
            }

            // Normal mode
            let prev_selected = state.selected;
            let page_size = (terminal.size()?.height as usize).saturating_sub(4) / 2;

            match key.code {
                KeyCode::Char('q') => break,
                KeyCode::Esc => {
                    if !state.search_query.is_empty() {
                        state.search_query.clear();
                        state.update_filter();
                        list_state.select(Some(state.selected));
                        state.load_keywords(db);
                    } else {
                        break;
                    }
                }
                KeyCode::Down | KeyCode::Char('j') => state.next(),
                KeyCode::Up | KeyCode::Char('k') => state.prev(),
                KeyCode::Char('d') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    state.page_down(page_size);
                }
                KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    state.page_up(page_size);
                }
                KeyCode::Char('g') => {
                    state.selected = 0;
                    state.scroll_offset = 0;
                }
                KeyCode::Char('G') => {
                    if !state.visible.is_empty() {
                        state.selected = state.visible.len() - 1;
                        state.scroll_offset = 0;
                    }
                }
                KeyCode::Char('J') | KeyCode::PageDown => state.scroll_down(),
                KeyCode::Char('K') | KeyCode::PageUp => state.scroll_up(),
                KeyCode::Char('/') => {
                    state.search_active = true;
                    state.search_query.clear();
                }
                KeyCode::Char('n') => state.update_current_job_status(db, "new"),
                KeyCode::Char('r') => state.update_current_job_status(db, "reviewing"),
                KeyCode::Char('a') => state.update_current_job_status(db, "applied"),
                KeyCode::Char('x') => state.update_current_job_status(db, "rejected"),
                KeyCode::Char('c') => state.update_current_job_status(db, "closed"),
                KeyCode::Char('1') => {
                    state.set_sort(SortField::Score);
                    list_state.select(Some(state.selected));
                    state.load_keywords(db);
                }
                KeyCode::Char('2') => {
                    state.set_sort(SortField::Salary);
                    list_state.select(Some(state.selected));
                    state.load_keywords(db);
                }
                KeyCode::Char('3') => {
                    state.set_sort(SortField::Fit);
                    list_state.select(Some(state.selected));
                    state.load_keywords(db);
                }
                KeyCode::Char('4') => {
                    state.set_sort(SortField::Company);
                    list_state.select(Some(state.selected));
                    state.load_keywords(db);
                }
                KeyCode::Char('H') => {
                    state.hide_closed = !state.hide_closed;
                    state.update_filter();
                    list_state.select(Some(state.selected));
                    state.load_keywords(db);
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

fn truncate_str(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else if max <= 2 {
        s.chars().take(max).collect()
    } else {
        let mut end = max - 2;
        while end > 0 && !s.is_char_boundary(end) {
            end -= 1;
        }
        format!("{}..", &s[..end])
    }
}

fn format_pay(job: &Job) -> String {
    let pay = job.pay_max.or(job.pay_min);
    match pay {
        Some(v) if v >= 1000 => format!("${:>3}k", v / 1000),
        Some(v) => format!("${:>4}", v),
        None => "   - ".to_string(),
    }
}

fn draw(frame: &mut Frame, state: &AppState, list_state: &mut ListState) {
    // Main layout: content + footer
    let main_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(0), Constraint::Length(1)])
        .split(frame.area());

    // Left/right split: 55% list / 45% detail
    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(55),
            Constraint::Percentage(45),
        ])
        .split(main_chunks[0]);

    // Compute column widths for job list
    // highlight symbol "> " = 2, borders = 2
    let usable = (chunks[0].width as usize).saturating_sub(4);
    // Format: "S #NNNN  85 $210k  Title                Employer"
    //          1 5      3  5      variable             variable
    // "S #NNNN SSS $NNNk " = status(1)+' '(1)+'#'(1)+id(4)+' '(1)+score(3)+' '(1)+pay(5)+' '(1) = 18
    let prefix_w = 18;
    let remaining = usable.saturating_sub(prefix_w);
    let emp_w = (remaining * 35 / 100).max(6).min(18);
    let title_w = remaining.saturating_sub(emp_w + 1); // +1 for space between title and employer

    // Left panel: job list
    let items: Vec<ListItem> = state.visible.iter().map(|&idx| {
        let job = &state.jobs[idx];
        let status_icon = match job.status.as_str() {
            "new" => " ",
            "reviewing" => "*",
            "applied" => "+",
            "rejected" => "x",
            "closed" => "-",
            _ => "?",
        };

        let score_str = match state.fit_scores[idx] {
            Some(s) => format!("{:>3.0}", s),
            None => "  -".to_string(),
        };

        let pay_str = format_pay(job);
        let employer = job.employer_name.as_deref().unwrap_or("?");
        let title = truncate_str(&job.title, title_w);
        let emp = truncate_str(employer, emp_w);

        let score_color = match state.fit_scores[idx] {
            Some(s) if s >= 75.0 => Color::Green,
            Some(s) if s >= 50.0 => Color::Yellow,
            Some(_) => Color::Red,
            None => Color::DarkGray,
        };

        ListItem::new(Line::from(vec![
            Span::raw(format!("{} #{:<4} ", status_icon, job.id)),
            Span::styled(score_str, Style::default().fg(score_color)),
            Span::styled(format!(" {} ", pay_str), Style::default().fg(Color::DarkGray)),
            Span::raw(format!("{:<width$}", title, width = title_w)),
            Span::styled(
                format!(" {:<width$}", emp, width = emp_w),
                Style::default().fg(Color::DarkGray),
            ),
        ]))
    }).collect();

    let sort_arrow = if state.sort_ascending { "\u{25b2}" } else { "\u{25bc}" };
    let sort_indicator = format!(" [{}{}]", state.sort_field.label(), sort_arrow);

    let list_title = if !state.search_query.is_empty() {
        format!(" Jobs ({}/{}) \"{}\"{} ", state.visible.len(), state.jobs.len(), state.search_query, sort_indicator)
    } else if state.visible.len() < state.jobs.len() {
        format!(" Jobs ({}/{}){} ", state.visible.len(), state.jobs.len(), sort_indicator)
    } else {
        format!(" Jobs ({}){} ", state.jobs.len(), sort_indicator)
    };

    let list = List::new(items)
        .block(Block::default().borders(Borders::ALL).title(list_title))
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

    // Footer
    let footer_text = if state.search_active {
        format!("/{}", state.search_query)
    } else {
        format!(" j/k:nav  ^D/^U:page  g/G:top/end  /:search  J/K:scroll  1-4:sort  n/r/a/x/c:status  H:{}  q:quit",
            if state.hide_closed { "show closed" } else { "hide closed" })
    };
    let footer_style = if state.search_active {
        Style::default().fg(Color::Yellow)
    } else {
        Style::default().fg(Color::DarkGray)
    };
    let footer = Paragraph::new(footer_text).style(footer_style);
    frame.render_widget(footer, main_chunks[1]);
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

    // Fit analysis summary
    if let Some(fit) = &state.fit_analysis {
        let score_color = if fit.fit_score >= 75.0 {
            Color::Green
        } else if fit.fit_score >= 50.0 {
            Color::Yellow
        } else {
            Color::Red
        };
        lines.push(Line::from(vec![
            Span::raw("Fit: "),
            Span::styled(format!("{:.0}/100", fit.fit_score), Style::default().fg(score_color).add_modifier(Modifier::BOLD)),
            Span::styled(format!(" ({})", fit.source_model), Style::default().fg(Color::DarkGray)),
        ]));

        if let Some(matches) = &fit.strong_matches {
            if !matches.is_empty() {
                lines.push(Line::from(Span::styled(
                    format!("  + {}", matches),
                    Style::default().fg(Color::Green),
                )));
            }
        }
        if let Some(gaps) = &fit.gaps {
            if !gaps.is_empty() {
                lines.push(Line::from(Span::styled(
                    format!("  - {}", gaps),
                    Style::default().fg(Color::Red),
                )));
            }
        }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_truncate_str_short() {
        assert_eq!(truncate_str("hello", 10), "hello");
    }

    #[test]
    fn test_truncate_str_exact() {
        assert_eq!(truncate_str("hello", 5), "hello");
    }

    #[test]
    fn test_truncate_str_long() {
        assert_eq!(truncate_str("hello world", 7), "hello..");
    }

    #[test]
    fn test_truncate_str_very_short_max() {
        assert_eq!(truncate_str("hello", 2), "he");
        assert_eq!(truncate_str("hello", 1), "h");
        assert_eq!(truncate_str("hello", 0), "");
    }

    #[test]
    fn test_truncate_str_empty() {
        assert_eq!(truncate_str("", 5), "");
    }

    #[test]
    fn test_format_pay_high() {
        let job = Job {
            id: 1, employer_id: None, employer_name: None,
            title: "Test".to_string(), url: None, source: None,
            status: "new".to_string(), raw_text: None,
            pay_min: Some(150000), pay_max: Some(200000),
            job_code: None, fetched_at: None, created_at: String::new(), updated_at: String::new(),
        };
        assert_eq!(format_pay(&job), "$200k");
    }

    #[test]
    fn test_format_pay_max_only() {
        let job = Job {
            id: 1, employer_id: None, employer_name: None,
            title: "Test".to_string(), url: None, source: None,
            status: "new".to_string(), raw_text: None,
            pay_min: None, pay_max: Some(175000),
            job_code: None, fetched_at: None, created_at: String::new(), updated_at: String::new(),
        };
        assert_eq!(format_pay(&job), "$175k");
    }

    #[test]
    fn test_format_pay_min_only() {
        let job = Job {
            id: 1, employer_id: None, employer_name: None,
            title: "Test".to_string(), url: None, source: None,
            status: "new".to_string(), raw_text: None,
            pay_min: Some(120000), pay_max: None,
            job_code: None, fetched_at: None, created_at: String::new(), updated_at: String::new(),
        };
        assert_eq!(format_pay(&job), "$120k");
    }

    #[test]
    fn test_format_pay_none() {
        let job = Job {
            id: 1, employer_id: None, employer_name: None,
            title: "Test".to_string(), url: None, source: None,
            status: "new".to_string(), raw_text: None,
            pay_min: None, pay_max: None,
            job_code: None, fetched_at: None, created_at: String::new(), updated_at: String::new(),
        };
        assert_eq!(format_pay(&job), "   - ");
    }

    #[test]
    fn test_format_pay_small_value() {
        let job = Job {
            id: 1, employer_id: None, employer_name: None,
            title: "Test".to_string(), url: None, source: None,
            status: "new".to_string(), raw_text: None,
            pay_min: None, pay_max: Some(500),
            job_code: None, fetched_at: None, created_at: String::new(), updated_at: String::new(),
        };
        assert_eq!(format_pay(&job), "$ 500");
    }

    #[test]
    fn test_sort_field_label() {
        assert_eq!(SortField::Score.label(), "Score");
        assert_eq!(SortField::Salary.label(), "Salary");
        assert_eq!(SortField::Fit.label(), "Fit");
        assert_eq!(SortField::Company.label(), "Company");
    }
}
