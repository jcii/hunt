use anyhow::{anyhow, Context, Result};
use rusqlite::{params, Connection};
use std::path::PathBuf;

use crate::models::{Employer, Job};

pub struct Database {
    conn: Connection,
    path: PathBuf,
}

impl Database {
    pub fn open() -> Result<Self> {
        let path = Self::default_path()?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let conn = Connection::open(&path)?;
        Ok(Self { conn, path })
    }

    pub fn path(&self) -> &PathBuf {
        &self.path
    }

    fn default_path() -> Result<PathBuf> {
        // Use XDG data directory or fallback
        if let Some(proj_dirs) = directories::ProjectDirs::from("", "", "hunt") {
            Ok(proj_dirs.data_dir().join("hunt.db"))
        } else {
            // Fallback to current directory
            Ok(PathBuf::from("hunt.db"))
        }
    }

    pub fn init(&self) -> Result<()> {
        self.conn.execute_batch(
            r#"
            CREATE TABLE IF NOT EXISTS employers (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                name TEXT NOT NULL UNIQUE,
                domain TEXT,
                status TEXT NOT NULL DEFAULT 'ok' CHECK (status IN ('ok', 'yuck', 'never')),
                notes TEXT,
                created_at TEXT NOT NULL DEFAULT (datetime('now')),
                updated_at TEXT NOT NULL DEFAULT (datetime('now'))
            );

            CREATE TABLE IF NOT EXISTS jobs (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                employer_id INTEGER REFERENCES employers(id),
                title TEXT NOT NULL,
                url TEXT,
                source TEXT,
                status TEXT NOT NULL DEFAULT 'new' CHECK (status IN ('new', 'reviewing', 'applied', 'rejected', 'closed')),
                pay_min INTEGER,
                pay_max INTEGER,
                raw_text TEXT,
                created_at TEXT NOT NULL DEFAULT (datetime('now')),
                updated_at TEXT NOT NULL DEFAULT (datetime('now'))
            );

            CREATE TABLE IF NOT EXISTS job_snapshots (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                job_id INTEGER NOT NULL REFERENCES jobs(id),
                raw_text TEXT NOT NULL,
                captured_at TEXT NOT NULL DEFAULT (datetime('now'))
            );

            CREATE INDEX IF NOT EXISTS idx_jobs_employer ON jobs(employer_id);
            CREATE INDEX IF NOT EXISTS idx_jobs_status ON jobs(status);
            CREATE INDEX IF NOT EXISTS idx_snapshots_job ON job_snapshots(job_id);
            "#,
        )?;
        Ok(())
    }

    pub fn ensure_initialized(&self) -> Result<()> {
        let tables: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='jobs'",
            [],
            |row| row.get(0),
        )?;
        if tables == 0 {
            return Err(anyhow!(
                "Database not initialized. Run 'hunt init' first."
            ));
        }
        Ok(())
    }

    // --- Employer operations ---

    pub fn get_or_create_employer(&self, name: &str) -> Result<i64> {
        // Try to find existing
        let existing: Option<i64> = self
            .conn
            .query_row(
                "SELECT id FROM employers WHERE LOWER(name) = LOWER(?1)",
                [name],
                |row| row.get(0),
            )
            .ok();

        if let Some(id) = existing {
            return Ok(id);
        }

        // Create new
        self.conn.execute(
            "INSERT INTO employers (name) VALUES (?1)",
            [name],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    pub fn list_employers(&self, status: Option<&str>) -> Result<Vec<Employer>> {
        let mut sql = String::from(
            "SELECT id, name, domain, status, notes, created_at, updated_at FROM employers",
        );
        if status.is_some() {
            sql.push_str(" WHERE status = ?1");
        }
        sql.push_str(" ORDER BY name");

        let mut stmt = self.conn.prepare(&sql)?;
        let rows = if let Some(s) = status {
            stmt.query_map([s], Self::row_to_employer)?
        } else {
            stmt.query_map([], Self::row_to_employer)?
        };

        rows.collect::<Result<Vec<_>, _>>()
            .context("Failed to list employers")
    }

    pub fn get_employer_by_name(&self, name: &str) -> Result<Option<Employer>> {
        let result = self.conn.query_row(
            "SELECT id, name, domain, status, notes, created_at, updated_at
             FROM employers WHERE LOWER(name) = LOWER(?1)",
            [name],
            Self::row_to_employer,
        );
        match result {
            Ok(emp) => Ok(Some(emp)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    pub fn set_employer_status(&self, name: &str, status: &str) -> Result<()> {
        // Create employer if doesn't exist
        let id = self.get_or_create_employer(name)?;
        self.conn.execute(
            "UPDATE employers SET status = ?1, updated_at = datetime('now') WHERE id = ?2",
            params![status, id],
        )?;
        Ok(())
    }

    fn row_to_employer(row: &rusqlite::Row) -> rusqlite::Result<Employer> {
        Ok(Employer {
            id: row.get(0)?,
            name: row.get(1)?,
            domain: row.get(2)?,
            status: row.get(3)?,
            notes: row.get(4)?,
            created_at: row.get(5)?,
            updated_at: row.get(6)?,
        })
    }

    // --- Job operations ---

    pub fn add_job(&self, content: &str) -> Result<i64> {
        // For now, just store the raw content as title and raw_text
        // TODO: Parse content to extract title, employer, pay, etc.
        let title = extract_title(content);
        let employer_name = extract_employer(content);

        let employer_id = if let Some(name) = &employer_name {
            Some(self.get_or_create_employer(name)?)
        } else {
            None
        };

        let (pay_min, pay_max) = extract_pay_range(content);

        self.conn.execute(
            "INSERT INTO jobs (employer_id, title, raw_text, pay_min, pay_max)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![employer_id, title, content, pay_min, pay_max],
        )?;

        let job_id = self.conn.last_insert_rowid();

        // Create initial snapshot
        self.conn.execute(
            "INSERT INTO job_snapshots (job_id, raw_text) VALUES (?1, ?2)",
            params![job_id, content],
        )?;

        Ok(job_id)
    }

    pub fn list_jobs(&self, status: Option<&str>, employer: Option<&str>) -> Result<Vec<Job>> {
        let mut sql = String::from(
            "SELECT j.id, j.employer_id, e.name, j.title, j.url, j.source, j.status,
                    j.pay_min, j.pay_max, j.raw_text, j.created_at, j.updated_at
             FROM jobs j
             LEFT JOIN employers e ON j.employer_id = e.id
             WHERE 1=1",
        );

        let mut params: Vec<String> = vec![];

        if let Some(s) = status {
            sql.push_str(&format!(" AND j.status = ?{}", params.len() + 1));
            params.push(s.to_string());
        }

        if let Some(emp) = employer {
            sql.push_str(&format!(" AND LOWER(e.name) = LOWER(?{})", params.len() + 1));
            params.push(emp.to_string());
        }

        sql.push_str(" ORDER BY j.created_at DESC");

        let mut stmt = self.conn.prepare(&sql)?;

        let rows = match params.len() {
            0 => stmt.query_map([], Self::row_to_job)?,
            1 => stmt.query_map([&params[0]], Self::row_to_job)?,
            2 => stmt.query_map([&params[0], &params[1]], Self::row_to_job)?,
            _ => return Err(anyhow!("Too many parameters")),
        };

        rows.collect::<Result<Vec<_>, _>>()
            .context("Failed to list jobs")
    }

    pub fn get_job(&self, id: i64) -> Result<Option<Job>> {
        let result = self.conn.query_row(
            "SELECT j.id, j.employer_id, e.name, j.title, j.url, j.source, j.status,
                    j.pay_min, j.pay_max, j.raw_text, j.created_at, j.updated_at
             FROM jobs j
             LEFT JOIN employers e ON j.employer_id = e.id
             WHERE j.id = ?1",
            [id],
            Self::row_to_job,
        );
        match result {
            Ok(job) => Ok(Some(job)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    pub fn rank_jobs(&self, limit: usize) -> Result<Vec<(Job, f64)>> {
        // Get all non-closed jobs
        let jobs = self.list_jobs(None, None)?;

        let mut scored: Vec<(Job, f64)> = jobs
            .into_iter()
            .filter(|j| j.status != "closed" && j.status != "rejected")
            .map(|job| {
                let score = calculate_score(&job, self);
                (job, score)
            })
            .collect();

        // Sort by score descending
        scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        scored.truncate(limit);

        Ok(scored)
    }

    fn row_to_job(row: &rusqlite::Row) -> rusqlite::Result<Job> {
        Ok(Job {
            id: row.get(0)?,
            employer_id: row.get(1)?,
            employer_name: row.get(2)?,
            title: row.get(3)?,
            url: row.get(4)?,
            source: row.get(5)?,
            status: row.get(6)?,
            pay_min: row.get(7)?,
            pay_max: row.get(8)?,
            raw_text: row.get(9)?,
            created_at: row.get(10)?,
            updated_at: row.get(11)?,
        })
    }

    pub fn get_employer_status(&self, employer_id: i64) -> Result<String> {
        let status: String = self.conn.query_row(
            "SELECT status FROM employers WHERE id = ?1",
            [employer_id],
            |row| row.get(0),
        )?;
        Ok(status)
    }
}

// --- Helper functions for parsing job content ---

fn extract_title(content: &str) -> String {
    // Take first line as title, or first 100 chars
    let first_line = content.lines().next().unwrap_or(content);
    if first_line.len() > 100 {
        format!("{}...", &first_line[..97])
    } else {
        first_line.to_string()
    }
}

fn extract_employer(content: &str) -> Option<String> {
    // Look for common patterns like "at Company" or "Company is hiring"
    let lower = content.to_lowercase();

    // Pattern: "at <Company>"
    if let Some(idx) = lower.find(" at ") {
        let after = &content[idx + 4..];
        let end = after.find(|c: char| c == '\n' || c == ',' || c == '-').unwrap_or(after.len());
        let company = after[..end].trim();
        if !company.is_empty() && company.len() < 50 {
            return Some(company.to_string());
        }
    }

    None
}

fn extract_pay_range(content: &str) -> (Option<i64>, Option<i64>) {
    // Look for salary patterns like "$150,000 - $200,000" or "$150k-200k"
    let re_patterns = [
        r"\$(\d{2,3}),?(\d{3})\s*[-–to]+\s*\$(\d{2,3}),?(\d{3})",  // $150,000 - $200,000
        r"\$(\d{2,3})k\s*[-–to]+\s*\$?(\d{2,3})k",                  // $150k - $200k
    ];

    // Simple pattern matching without regex for now
    let lower = content.to_lowercase();

    // Look for "$XXXk" patterns
    let mut pay_min = None;
    let mut pay_max = None;

    let chars: Vec<char> = lower.chars().collect();
    for i in 0..chars.len() {
        if chars[i] == '$' {
            // Try to parse number after $
            let mut j = i + 1;
            let mut num_str = String::new();
            while j < chars.len() && (chars[j].is_ascii_digit() || chars[j] == ',' || chars[j] == '.') {
                if chars[j].is_ascii_digit() {
                    num_str.push(chars[j]);
                }
                j += 1;
            }

            if !num_str.is_empty() {
                if let Ok(num) = num_str.parse::<i64>() {
                    let value = if j < chars.len() && chars[j] == 'k' {
                        num * 1000
                    } else if num < 1000 {
                        // Likely already in thousands (e.g., $150 meaning $150k)
                        num * 1000
                    } else {
                        num
                    };

                    if pay_min.is_none() {
                        pay_min = Some(value);
                    } else if pay_max.is_none() {
                        pay_max = Some(value);
                    }
                }
            }
        }
    }

    // Ensure min < max
    if let (Some(min), Some(max)) = (pay_min, pay_max) {
        if min > max {
            return (Some(max), Some(min));
        }
    }

    (pay_min, pay_max)
}

fn calculate_score(job: &Job, db: &Database) -> f64 {
    let mut score = 50.0; // Base score

    // Pay bonus (higher pay = higher score)
    if let Some(max) = job.pay_max {
        score += (max as f64 / 10000.0).min(30.0); // Up to 30 points for high pay
    } else if let Some(min) = job.pay_min {
        score += (min as f64 / 15000.0).min(20.0); // Up to 20 points if only min
    }

    // Employer status penalty
    if let Some(emp_id) = job.employer_id {
        if let Ok(status) = db.get_employer_status(emp_id) {
            match status.as_str() {
                "yuck" => score -= 20.0,
                "never" => score -= 100.0, // Should effectively exclude
                _ => {}
            }
        }
    }

    // Status bonus (reviewing > new)
    match job.status.as_str() {
        "reviewing" => score += 10.0,
        "new" => score += 5.0,
        _ => {}
    }

    score.max(0.0)
}
