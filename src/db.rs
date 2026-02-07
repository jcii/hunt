use anyhow::{anyhow, Context, Result};
use rusqlite::{params, Connection};
use std::path::PathBuf;

use crate::models::{BaseResume, Employer, GlassdoorReview, Job, ResumeVariant};

pub struct DestructionStats {
    pub jobs: i64,
    pub job_snapshots: i64,
    pub employers: i64,
    pub base_resumes: i64,
    pub resume_variants: i64,
}

impl DestructionStats {
    pub fn total(&self) -> i64 {
        self.jobs + self.job_snapshots + self.employers + self.base_resumes + self.resume_variants
    }
}

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
                updated_at TEXT NOT NULL DEFAULT (datetime('now')),
                crunchbase_url TEXT,
                funding_stage TEXT,
                total_funding INTEGER,
                last_funding_date TEXT,
                yc_batch TEXT,
                yc_url TEXT,
                hn_mentions_count INTEGER,
                recent_news TEXT,
                research_updated_at TEXT,
                controversies TEXT,
                labor_practices TEXT,
                environmental_issues TEXT,
                political_donations TEXT,
                evil_summary TEXT,
                public_research_updated_at TEXT,
                parent_company TEXT,
                pe_owner TEXT,
                pe_firm_url TEXT,
                vc_investors TEXT,
                key_investors TEXT,
                ownership_concerns TEXT,
                ownership_type TEXT,
                ownership_research_updated TEXT
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
                job_code TEXT,
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

            CREATE TABLE IF NOT EXISTS base_resumes (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                name TEXT NOT NULL UNIQUE,
                format TEXT NOT NULL CHECK (format IN ('markdown', 'plain', 'json', 'latex')),
                content TEXT NOT NULL,
                notes TEXT,
                created_at TEXT NOT NULL DEFAULT (datetime('now')),
                updated_at TEXT NOT NULL DEFAULT (datetime('now'))
            );

            CREATE TABLE IF NOT EXISTS resume_variants (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                base_resume_id INTEGER NOT NULL REFERENCES base_resumes(id),
                job_id INTEGER NOT NULL REFERENCES jobs(id),
                content TEXT NOT NULL,
                tailoring_notes TEXT,
                created_at TEXT NOT NULL DEFAULT (datetime('now')),
                UNIQUE(base_resume_id, job_id)
            );

            CREATE INDEX IF NOT EXISTS idx_variants_base ON resume_variants(base_resume_id);
            CREATE INDEX IF NOT EXISTS idx_variants_job ON resume_variants(job_id);

            CREATE TABLE IF NOT EXISTS glassdoor_reviews (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                employer_id INTEGER NOT NULL REFERENCES employers(id),
                rating REAL NOT NULL,
                title TEXT,
                pros TEXT,
                cons TEXT,
                review_text TEXT,
                sentiment TEXT NOT NULL CHECK (sentiment IN ('positive', 'negative', 'neutral')),
                review_date TEXT,
                captured_at TEXT NOT NULL DEFAULT (datetime('now'))
            );

            CREATE INDEX IF NOT EXISTS idx_glassdoor_employer ON glassdoor_reviews(employer_id);
            CREATE INDEX IF NOT EXISTS idx_glassdoor_date ON glassdoor_reviews(review_date);
            "#,
        )?;

        // Run migrations for existing databases
        self.migrate()?;

        Ok(())
    }

    fn migrate(&self) -> Result<()> {
        // Check if startup research columns exist
        let columns: Vec<String> = self.conn
            .prepare("PRAGMA table_info(employers)")?
            .query_map([], |row| row.get::<_, String>(1))?
            .collect::<Result<Vec<_>, _>>()?;

        if !columns.contains(&"crunchbase_url".to_string()) {
            self.conn.execute_batch(
                r#"
                ALTER TABLE employers ADD COLUMN crunchbase_url TEXT;
                ALTER TABLE employers ADD COLUMN funding_stage TEXT;
                ALTER TABLE employers ADD COLUMN total_funding INTEGER;
                ALTER TABLE employers ADD COLUMN last_funding_date TEXT;
                ALTER TABLE employers ADD COLUMN yc_batch TEXT;
                ALTER TABLE employers ADD COLUMN yc_url TEXT;
                ALTER TABLE employers ADD COLUMN hn_mentions_count INTEGER;
                ALTER TABLE employers ADD COLUMN recent_news TEXT;
                ALTER TABLE employers ADD COLUMN research_updated_at TEXT;
                "#,
            )?;
        }

        // Check if public company research columns exist
        if !columns.contains(&"controversies".to_string()) {
            self.conn.execute_batch(
                r#"
                ALTER TABLE employers ADD COLUMN controversies TEXT;
                ALTER TABLE employers ADD COLUMN labor_practices TEXT;
                ALTER TABLE employers ADD COLUMN environmental_issues TEXT;
                ALTER TABLE employers ADD COLUMN political_donations TEXT;
                ALTER TABLE employers ADD COLUMN evil_summary TEXT;
                ALTER TABLE employers ADD COLUMN public_research_updated_at TEXT;
                "#,
            )?;
        }

        // Check if private company ownership columns exist
        if !columns.contains(&"parent_company".to_string()) {
            self.conn.execute_batch(
                r#"
                ALTER TABLE employers ADD COLUMN parent_company TEXT;
                ALTER TABLE employers ADD COLUMN pe_owner TEXT;
                ALTER TABLE employers ADD COLUMN pe_firm_url TEXT;
                ALTER TABLE employers ADD COLUMN vc_investors TEXT;
                ALTER TABLE employers ADD COLUMN key_investors TEXT;
                ALTER TABLE employers ADD COLUMN ownership_concerns TEXT;
                ALTER TABLE employers ADD COLUMN ownership_type TEXT;
                ALTER TABLE employers ADD COLUMN ownership_research_updated TEXT;
                "#,
            )?;
        }

        // Check if job_code column exists in jobs table
        let job_columns: Vec<String> = self.conn
            .prepare("PRAGMA table_info(jobs)")?
            .query_map([], |row| row.get::<_, String>(1))?
            .collect::<Result<Vec<_>, _>>()?;

        if !job_columns.contains(&"job_code".to_string()) {
            self.conn.execute(
                "ALTER TABLE jobs ADD COLUMN job_code TEXT",
                [],
            )?;
        }

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
            "SELECT id, name, domain, status, notes, created_at, updated_at,
             crunchbase_url, funding_stage, total_funding, last_funding_date,
             yc_batch, yc_url, hn_mentions_count, recent_news, research_updated_at,
             controversies, labor_practices, environmental_issues, political_donations,
             evil_summary, public_research_updated_at,
             parent_company, pe_owner, pe_firm_url, vc_investors, key_investors,
             ownership_concerns, ownership_type, ownership_research_updated
             FROM employers",
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
            "SELECT id, name, domain, status, notes, created_at, updated_at,
             crunchbase_url, funding_stage, total_funding, last_funding_date,
             yc_batch, yc_url, hn_mentions_count, recent_news, research_updated_at,
             controversies, labor_practices, environmental_issues, political_donations,
             evil_summary, public_research_updated_at,
             parent_company, pe_owner, pe_firm_url, vc_investors, key_investors,
             ownership_concerns, ownership_type, ownership_research_updated
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

    pub fn update_employer_research(
        &self,
        employer_id: i64,
        crunchbase_url: Option<&str>,
        funding_stage: Option<&str>,
        total_funding: Option<i64>,
        last_funding_date: Option<&str>,
        yc_batch: Option<&str>,
        yc_url: Option<&str>,
        hn_mentions_count: Option<i64>,
        recent_news: Option<&str>,
    ) -> Result<()> {
        self.conn.execute(
            "UPDATE employers SET
                crunchbase_url = ?1,
                funding_stage = ?2,
                total_funding = ?3,
                last_funding_date = ?4,
                yc_batch = ?5,
                yc_url = ?6,
                hn_mentions_count = ?7,
                recent_news = ?8,
                research_updated_at = datetime('now'),
                updated_at = datetime('now')
             WHERE id = ?9",
            params![
                crunchbase_url,
                funding_stage,
                total_funding,
                last_funding_date,
                yc_batch,
                yc_url,
                hn_mentions_count,
                recent_news,
                employer_id
            ],
        )?;
        Ok(())
    }

    pub fn update_public_company_research(
        &self,
        employer_id: i64,
        controversies: Option<&str>,
        labor_practices: Option<&str>,
        environmental_issues: Option<&str>,
        political_donations: Option<&str>,
        evil_summary: Option<&str>,
    ) -> Result<()> {
        self.conn.execute(
            "UPDATE employers SET
                controversies = ?1,
                labor_practices = ?2,
                environmental_issues = ?3,
                political_donations = ?4,
                evil_summary = ?5,
                public_research_updated_at = datetime('now'),
                updated_at = datetime('now')
             WHERE id = ?6",
            params![
                controversies,
                labor_practices,
                environmental_issues,
                political_donations,
                evil_summary,
                employer_id
            ],
        )?;
        Ok(())
    }

    pub fn update_employer_ownership(
        &self,
        employer_id: i64,
        parent_company: Option<&str>,
        pe_owner: Option<&str>,
        pe_firm_url: Option<&str>,
        vc_investors: Option<&str>,
        key_investors: Option<&str>,
        ownership_concerns: Option<&str>,
        ownership_type: Option<&str>,
    ) -> Result<()> {
        self.conn.execute(
            "UPDATE employers SET
                parent_company = ?1,
                pe_owner = ?2,
                pe_firm_url = ?3,
                vc_investors = ?4,
                key_investors = ?5,
                ownership_concerns = ?6,
                ownership_type = ?7,
                ownership_research_updated = datetime('now'),
                updated_at = datetime('now')
             WHERE id = ?8",
            params![
                parent_company,
                pe_owner,
                pe_firm_url,
                vc_investors,
                key_investors,
                ownership_concerns,
                ownership_type,
                employer_id
            ],
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
            crunchbase_url: row.get(7)?,
            funding_stage: row.get(8)?,
            total_funding: row.get(9)?,
            last_funding_date: row.get(10)?,
            yc_batch: row.get(11)?,
            yc_url: row.get(12)?,
            hn_mentions_count: row.get(13)?,
            recent_news: row.get(14)?,
            research_updated_at: row.get(15)?,
            controversies: row.get(16)?,
            labor_practices: row.get(17)?,
            environmental_issues: row.get(18)?,
            political_donations: row.get(19)?,
            evil_summary: row.get(20)?,
            public_research_updated_at: row.get(21)?,
            parent_company: row.get(22)?,
            pe_owner: row.get(23)?,
            pe_firm_url: row.get(24)?,
            vc_investors: row.get(25)?,
            key_investors: row.get(26)?,
            ownership_concerns: row.get(27)?,
            ownership_type: row.get(28)?,
            ownership_research_updated: row.get(29)?,
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
        let job_code = extract_job_code(content);

        self.conn.execute(
            "INSERT INTO jobs (employer_id, title, raw_text, pay_min, pay_max, job_code)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![employer_id, title, content, pay_min, pay_max, job_code],
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
                    j.pay_min, j.pay_max, j.job_code, j.raw_text, j.created_at, j.updated_at
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

        sql.push_str(" ORDER BY j.id ASC");

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
                    j.pay_min, j.pay_max, j.job_code, j.raw_text, j.created_at, j.updated_at
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

    pub fn get_jobs_without_descriptions(&self, limit: Option<usize>, force: bool) -> Result<Vec<Job>> {
        let where_clause = if force {
            "j.url IS NOT NULL"
        } else {
            "j.raw_text IS NULL AND j.url IS NOT NULL"
        };

        let query = if let Some(lim) = limit {
            format!(
                "SELECT j.id, j.employer_id, e.name, j.title, j.url, j.source, j.status,
                        j.pay_min, j.pay_max, j.job_code, j.raw_text, j.created_at, j.updated_at
                 FROM jobs j
                 LEFT JOIN employers e ON j.employer_id = e.id
                 WHERE {}
                 ORDER BY j.created_at ASC
                 LIMIT {}",
                where_clause, lim
            )
        } else {
            format!(
                "SELECT j.id, j.employer_id, e.name, j.title, j.url, j.source, j.status,
                        j.pay_min, j.pay_max, j.job_code, j.raw_text, j.created_at, j.updated_at
                 FROM jobs j
                 LEFT JOIN employers e ON j.employer_id = e.id
                 WHERE {}
                 ORDER BY j.created_at ASC",
                where_clause
            )
        };

        let mut stmt = self.conn.prepare(&query)?;
        let jobs = stmt
            .query_map([], Self::row_to_job)?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(jobs)
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
            job_code: row.get(9)?,
            raw_text: row.get(10)?,
            created_at: row.get(11)?,
            updated_at: row.get(12)?,
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

    pub fn delete_job(&self, id: i64) -> Result<()> {
        // Delete associated snapshots first (foreign key constraint)
        self.conn.execute(
            "DELETE FROM job_snapshots WHERE job_id = ?1",
            [id],
        )?;

        // Delete resume variants for this job
        self.conn.execute(
            "DELETE FROM resume_variants WHERE job_id = ?1",
            [id],
        )?;

        // Delete the job
        self.conn.execute(
            "DELETE FROM jobs WHERE id = ?1",
            [id],
        )?;
        Ok(())
    }

    // --- Email ingestion support ---

    #[allow(dead_code)]
    pub fn job_exists_by_url(&self, url: &str) -> Result<bool> {
        let count: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM jobs WHERE url = ?1",
            [url],
            |row| row.get(0),
        )?;
        Ok(count > 0)
    }

    #[allow(dead_code)]
    pub fn job_exists_by_title_employer(&self, title: &str, employer: &str) -> Result<bool> {
        let count: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM jobs j
             JOIN employers e ON j.employer_id = e.id
             WHERE LOWER(j.title) = LOWER(?1) AND LOWER(e.name) = LOWER(?2)",
            params![title, employer],
            |row| row.get(0),
        )?;
        Ok(count > 0)
    }

    /// Check if a job is a duplicate using sophisticated deduplication rules
    pub fn is_duplicate_job(
        &self,
        title: &str,
        employer: Option<&str>,
        url: Option<&str>,
    ) -> Result<Option<i64>> {
        // Rule 1: Check by URL if present (exact match)
        if let Some(url) = url {
            let result: Option<i64> = self
                .conn
                .query_row(
                    "SELECT id FROM jobs WHERE url = ?1",
                    [url],
                    |row| row.get(0),
                )
                .ok();
            if result.is_some() {
                return Ok(result);
            }
        }

        // Rules 2-4: Check by title similarity with same employer
        if let Some(employer) = employer {
            // Get all jobs from this employer
            let mut stmt = self.conn.prepare(
                "SELECT j.id, j.title
                 FROM jobs j
                 JOIN employers e ON j.employer_id = e.id
                 WHERE LOWER(e.name) = LOWER(?1)",
            )?;

            let jobs = stmt.query_map([employer], |row| {
                Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
            })?;

            let title_normalized = normalize_title(title);

            for job_result in jobs {
                let (job_id, existing_title) = job_result?;
                let existing_normalized = normalize_title(&existing_title);

                // Rule 2: Exact match (case-insensitive, normalized)
                if title_normalized == existing_normalized {
                    return Ok(Some(job_id));
                }

                // Rule 3: Substring match - if new title is substring of existing or vice versa
                if existing_normalized.contains(&title_normalized)
                    || title_normalized.contains(&existing_normalized)
                {
                    return Ok(Some(job_id));
                }

                // Rule 4: Fuzzy match - >80% similar
                let similarity = strsim::jaro_winkler(&title_normalized, &existing_normalized);
                if similarity > 0.8 {
                    return Ok(Some(job_id));
                }
            }
        }

        Ok(None)
    }

    /// Find and return all duplicate jobs
    pub fn find_duplicates(&self) -> Result<Vec<(i64, i64, String)>> {
        let mut duplicates = Vec::new();

        // Get all jobs with their employer info
        let mut stmt = self.conn.prepare(
            "SELECT j.id, j.title, j.url, e.name, j.created_at
             FROM jobs j
             LEFT JOIN employers e ON j.employer_id = e.id
             ORDER BY j.created_at ASC",
        )?;

        let jobs: Vec<(i64, String, Option<String>, Option<String>, String)> = stmt
            .query_map([], |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                ))
            })?
            .collect::<Result<Vec<_>, _>>()?;

        // Check each job against earlier jobs
        for i in 1..jobs.len() {
            let (job_id, title, url, employer, _) = &jobs[i];

            for j in 0..i {
                let (earlier_id, earlier_title, earlier_url, earlier_employer, _) = &jobs[j];

                // Skip if already marked as duplicate
                if duplicates.iter().any(|(_, dup_id, _)| dup_id == job_id) {
                    continue;
                }

                // Check if this is a duplicate
                let is_dup = if let (Some(url), Some(earlier_url)) = (url, earlier_url) {
                    // URL match
                    url == earlier_url
                } else if let (Some(emp), Some(earlier_emp)) = (employer, earlier_employer) {
                    if emp.to_lowercase() == earlier_emp.to_lowercase() {
                        let title_norm = normalize_title(title);
                        let earlier_norm = normalize_title(earlier_title);

                        // Same employer - check title similarity
                        title_norm == earlier_norm
                            || title_norm.contains(&earlier_norm)
                            || earlier_norm.contains(&title_norm)
                            || strsim::jaro_winkler(&title_norm, &earlier_norm) > 0.8
                    } else {
                        false
                    }
                } else {
                    false
                };

                if is_dup {
                    duplicates.push((
                        *earlier_id,
                        *job_id,
                        format!(
                            "Job #{} ('{}') duplicates job #{} ('{}')",
                            job_id, title, earlier_id, earlier_title
                        ),
                    ));
                    break;
                }
            }
        }

        Ok(duplicates)
    }

    pub fn add_job_full(
        &self,
        title: &str,
        employer: Option<&str>,
        url: Option<&str>,
        source: Option<&str>,
        pay_min: Option<i64>,
        pay_max: Option<i64>,
        raw_text: Option<&str>,
    ) -> Result<i64> {
        let employer_id = if let Some(name) = employer {
            Some(self.get_or_create_employer(name)?)
        } else {
            None
        };

        // Extract job code from raw text if available
        let job_code = raw_text.and_then(|text| extract_job_code(text));

        self.conn.execute(
            "INSERT INTO jobs (employer_id, title, url, source, pay_min, pay_max, job_code, raw_text)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![employer_id, title, url, source, pay_min, pay_max, job_code, raw_text],
        )?;

        let job_id = self.conn.last_insert_rowid();

        // Create initial snapshot if we have raw text
        if let Some(text) = raw_text {
            self.conn.execute(
                "INSERT INTO job_snapshots (job_id, raw_text) VALUES (?1, ?2)",
                params![job_id, text],
            )?;
        }

        Ok(job_id)
    }

    pub fn update_job_description(&self, job_id: i64, description: &str, pay_min: Option<i64>, pay_max: Option<i64>) -> Result<()> {
        self.conn.execute(
            "UPDATE jobs
             SET raw_text = ?1, pay_min = ?2, pay_max = ?3, updated_at = datetime('now')
             WHERE id = ?4",
            params![description, pay_min, pay_max, job_id],
        )?;

        // Create a snapshot of the new description
        self.conn.execute(
            "INSERT INTO job_snapshots (job_id, raw_text) VALUES (?1, ?2)",
            params![job_id, description],
        )?;

        Ok(())
    }

    // --- Base Resume operations ---

    pub fn create_base_resume(
        &self,
        name: &str,
        format: &str,
        content: &str,
        notes: Option<&str>,
    ) -> Result<i64> {
        self.conn.execute(
            "INSERT INTO base_resumes (name, format, content, notes)
             VALUES (?1, ?2, ?3, ?4)",
            params![name, format, content, notes],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    pub fn list_base_resumes(&self) -> Result<Vec<BaseResume>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, name, format, content, notes, created_at, updated_at
             FROM base_resumes
             ORDER BY updated_at DESC",
        )?;

        let rows = stmt.query_map([], |row| {
            Ok(BaseResume {
                id: row.get(0)?,
                name: row.get(1)?,
                format: row.get(2)?,
                content: row.get(3)?,
                notes: row.get(4)?,
                created_at: row.get(5)?,
                updated_at: row.get(6)?,
            })
        })?;

        rows.collect::<Result<Vec<_>, _>>()
            .context("Failed to list base resumes")
    }

    pub fn get_base_resume(&self, id: i64) -> Result<Option<BaseResume>> {
        let result = self.conn.query_row(
            "SELECT id, name, format, content, notes, created_at, updated_at
             FROM base_resumes WHERE id = ?1",
            [id],
            |row| {
                Ok(BaseResume {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    format: row.get(2)?,
                    content: row.get(3)?,
                    notes: row.get(4)?,
                    created_at: row.get(5)?,
                    updated_at: row.get(6)?,
                })
            },
        );
        match result {
            Ok(resume) => Ok(Some(resume)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    pub fn get_base_resume_by_name(&self, name: &str) -> Result<Option<BaseResume>> {
        let result = self.conn.query_row(
            "SELECT id, name, format, content, notes, created_at, updated_at
             FROM base_resumes WHERE name = ?1",
            [name],
            |row| {
                Ok(BaseResume {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    format: row.get(2)?,
                    content: row.get(3)?,
                    notes: row.get(4)?,
                    created_at: row.get(5)?,
                    updated_at: row.get(6)?,
                })
            },
        );
        match result {
            Ok(resume) => Ok(Some(resume)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    #[allow(dead_code)]
    pub fn update_base_resume(
        &self,
        id: i64,
        name: Option<&str>,
        format: Option<&str>,
        content: Option<&str>,
        notes: Option<&str>,
    ) -> Result<()> {
        let mut updates = Vec::new();
        let mut params: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();

        if let Some(n) = name {
            updates.push("name = ?");
            params.push(Box::new(n.to_string()));
        }
        if let Some(f) = format {
            updates.push("format = ?");
            params.push(Box::new(f.to_string()));
        }
        if let Some(c) = content {
            updates.push("content = ?");
            params.push(Box::new(c.to_string()));
        }
        if let Some(n) = notes {
            updates.push("notes = ?");
            params.push(Box::new(n.to_string()));
        }

        if updates.is_empty() {
            return Ok(());
        }

        updates.push("updated_at = datetime('now')");
        params.push(Box::new(id));

        let sql = format!(
            "UPDATE base_resumes SET {} WHERE id = ?",
            updates.join(", ")
        );

        let params_ref: Vec<&dyn rusqlite::ToSql> = params.iter().map(|p| p.as_ref()).collect();
        self.conn.execute(&sql, params_ref.as_slice())?;
        Ok(())
    }

    // --- Resume Variant operations ---

    pub fn create_resume_variant(
        &self,
        base_resume_id: i64,
        job_id: i64,
        content: &str,
        tailoring_notes: Option<&str>,
    ) -> Result<i64> {
        self.conn.execute(
            "INSERT INTO resume_variants (base_resume_id, job_id, content, tailoring_notes)
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(base_resume_id, job_id) DO UPDATE SET
                content = excluded.content,
                tailoring_notes = excluded.tailoring_notes",
            params![base_resume_id, job_id, content, tailoring_notes],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    #[allow(dead_code)]
    pub fn get_resume_variant(&self, job_id: i64, base_resume_id: i64) -> Result<Option<ResumeVariant>> {
        let result = self.conn.query_row(
            "SELECT id, base_resume_id, job_id, content, tailoring_notes, created_at
             FROM resume_variants WHERE job_id = ?1 AND base_resume_id = ?2",
            params![job_id, base_resume_id],
            |row| {
                Ok(ResumeVariant {
                    id: row.get(0)?,
                    base_resume_id: row.get(1)?,
                    job_id: row.get(2)?,
                    content: row.get(3)?,
                    tailoring_notes: row.get(4)?,
                    created_at: row.get(5)?,
                })
            },
        );
        match result {
            Ok(variant) => Ok(Some(variant)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    pub fn list_resume_variants_for_job(&self, job_id: i64) -> Result<Vec<ResumeVariant>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, base_resume_id, job_id, content, tailoring_notes, created_at
             FROM resume_variants WHERE job_id = ?1
             ORDER BY created_at DESC",
        )?;

        let rows = stmt.query_map([job_id], |row| {
            Ok(ResumeVariant {
                id: row.get(0)?,
                base_resume_id: row.get(1)?,
                job_id: row.get(2)?,
                content: row.get(3)?,
                tailoring_notes: row.get(4)?,
                created_at: row.get(5)?,
            })
        })?;

        rows.collect::<Result<Vec<_>, _>>()
            .context("Failed to list resume variants")
    }

    // --- Destruction operations ---

    pub fn get_destruction_stats(&self) -> Result<DestructionStats> {
        let jobs: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM jobs",
            [],
            |row| row.get(0),
        )?;

        let job_snapshots: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM job_snapshots",
            [],
            |row| row.get(0),
        )?;

        let employers: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM employers",
            [],
            |row| row.get(0),
        )?;

        let base_resumes: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM base_resumes",
            [],
            |row| row.get(0),
        )?;

        let resume_variants: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM resume_variants",
            [],
            |row| row.get(0),
        )?;

        Ok(DestructionStats {
            jobs,
            job_snapshots,
            employers,
            base_resumes,
            resume_variants,
        })
    }

    pub fn destroy_all_data(&self) -> Result<()> {
        // Delete all data from all tables
        self.conn.execute("DELETE FROM resume_variants", [])?;
        self.conn.execute("DELETE FROM base_resumes", [])?;
        self.conn.execute("DELETE FROM job_snapshots", [])?;
        self.conn.execute("DELETE FROM glassdoor_reviews", [])?;
        self.conn.execute("DELETE FROM jobs", [])?;
        self.conn.execute("DELETE FROM employers", [])?;

        // Reset auto-increment counters
        self.conn.execute("DELETE FROM sqlite_sequence", [])?;

        Ok(())
    }

    // --- Glassdoor Review operations ---

    pub fn add_glassdoor_review(
        &self,
        employer_id: i64,
        rating: f64,
        title: Option<&str>,
        pros: Option<&str>,
        cons: Option<&str>,
        review_text: Option<&str>,
        sentiment: &str,
        review_date: Option<&str>,
    ) -> Result<i64> {
        self.conn.execute(
            "INSERT INTO glassdoor_reviews
             (employer_id, rating, title, pros, cons, review_text, sentiment, review_date)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![employer_id, rating, title, pros, cons, review_text, sentiment, review_date],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    pub fn list_glassdoor_reviews(&self, employer_id: Option<i64>) -> Result<Vec<GlassdoorReview>> {
        let mut sql = String::from(
            "SELECT r.id, r.employer_id, e.name, r.rating, r.title, r.pros, r.cons,
                    r.review_text, r.sentiment, r.review_date, r.captured_at
             FROM glassdoor_reviews r
             JOIN employers e ON r.employer_id = e.id",
        );

        if employer_id.is_some() {
            sql.push_str(" WHERE r.employer_id = ?1");
        }
        sql.push_str(" ORDER BY r.review_date DESC, r.captured_at DESC");

        let mut stmt = self.conn.prepare(&sql)?;
        let rows = if let Some(id) = employer_id {
            stmt.query_map([id], Self::row_to_glassdoor_review)?
        } else {
            stmt.query_map([], Self::row_to_glassdoor_review)?
        };

        rows.collect::<Result<Vec<_>, _>>()
            .context("Failed to list Glassdoor reviews")
    }

    pub fn get_recent_review_count(&self, employer_id: i64, since: &str) -> Result<i64> {
        let count: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM glassdoor_reviews
             WHERE employer_id = ?1 AND review_date >= ?2",
            params![employer_id, since],
            |row| row.get(0),
        )?;
        Ok(count)
    }

    pub fn get_sentiment_summary(&self, employer_id: i64) -> Result<(i64, i64, i64, f64)> {
        let positive: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM glassdoor_reviews
             WHERE employer_id = ?1 AND sentiment = 'positive'",
            [employer_id],
            |row| row.get(0),
        )?;

        let negative: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM glassdoor_reviews
             WHERE employer_id = ?1 AND sentiment = 'negative'",
            [employer_id],
            |row| row.get(0),
        )?;

        let neutral: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM glassdoor_reviews
             WHERE employer_id = ?1 AND sentiment = 'neutral'",
            [employer_id],
            |row| row.get(0),
        )?;

        let avg_rating: f64 = self.conn.query_row(
            "SELECT COALESCE(AVG(rating), 0.0) FROM glassdoor_reviews
             WHERE employer_id = ?1",
            [employer_id],
            |row| row.get(0),
        )?;

        Ok((positive, negative, neutral, avg_rating))
    }

    fn row_to_glassdoor_review(row: &rusqlite::Row) -> rusqlite::Result<GlassdoorReview> {
        Ok(GlassdoorReview {
            id: row.get(0)?,
            employer_id: row.get(1)?,
            employer_name: row.get(2)?,
            rating: row.get(3)?,
            title: row.get(4)?,
            pros: row.get(5)?,
            cons: row.get(6)?,
            review_text: row.get(7)?,
            sentiment: row.get(8)?,
            review_date: row.get(9)?,
            captured_at: row.get(10)?,
        })
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

fn extract_job_code(content: &str) -> Option<String> {
    // Common job code patterns:
    // - "Job ID: 12345"
    // - "Job Code: ABC123"
    // - "Requisition ID: REQ-2024-001"
    // - "Req#: 123456"
    // - "Job #: 987654"
    // - "Job Number: 12345"
    // - "JR12345" or "R12345" (common LinkedIn format)

    let lower = content.to_lowercase();
    let patterns = [
        ("job id:", 7),
        ("job code:", 10),
        ("requisition id:", 15),
        ("req id:", 7),
        ("req#:", 5),
        ("req #:", 6),
        ("job #:", 6),
        ("job number:", 11),
        ("job no:", 7),
        ("reference:", 10),
        ("ref:", 4),
    ];

    // Try each pattern
    for (pattern, offset) in patterns {
        if let Some(idx) = lower.find(pattern) {
            let after = &content[idx + offset..];
            // Extract code (alphanumeric, dashes, underscores)
            let code: String = after
                .chars()
                .skip_while(|c| c.is_whitespace())
                .take_while(|c| c.is_alphanumeric() || *c == '-' || *c == '_' || *c == '/')
                .collect();

            if !code.is_empty() && code.len() <= 50 {
                return Some(code);
            }
        }
    }

    // Look for LinkedIn job ID pattern in URL (job/view/123456)
    if let Some(idx) = content.find("/job/view/") {
        let after = &content[idx + 10..];
        let id: String = after
            .chars()
            .take_while(|c| c.is_ascii_digit())
            .collect();
        if !id.is_empty() {
            return Some(format!("linkedin-{}", id));
        }
    }

    // Look for "JR" or "R" followed by numbers (common format)
    if let Some(idx) = content.find("JR") {
        let after = &content[idx + 2..];
        let code: String = after
            .chars()
            .take_while(|c| c.is_alphanumeric() || *c == '-')
            .collect();
        if !code.is_empty() && code.len() >= 4 && code.len() <= 20 {
            return Some(format!("JR{}", code));
        }
    }

    None
}

pub fn extract_pay_range(content: &str) -> (Option<i64>, Option<i64>) {
    // Look for salary patterns like "$150,000 - $200,000" or "$150k-200k"
    let _re_patterns = [
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

/// Normalize title for comparison: trim and lowercase
fn normalize_title(title: &str) -> String {
    title.trim().to_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_test_db() -> Result<Database> {
        let conn = Connection::open_in_memory()?;
        let db = Database {
            conn,
            path: PathBuf::from(":memory:"),
        };
        db.init()?;
        Ok(db)
    }

    #[test]
    fn test_exact_title_match_same_employer() -> Result<()> {
        let db = create_test_db()?;

        // Add first job
        db.add_job_full(
            "Staff DevOps Engineer",
            Some("Wiraa"),
            None,
            Some("linkedin"),
            None,
            None,
            None,
        )?;

        // Check for duplicate with exact same title and employer
        let duplicate = db.is_duplicate_job("Staff DevOps Engineer", Some("Wiraa"), None)?;
        assert!(duplicate.is_some(), "Exact match should be detected as duplicate");

        Ok(())
    }

    #[test]
    fn test_substring_match_same_employer() -> Result<()> {
        let db = create_test_db()?;

        // Add job with longer title
        db.add_job_full(
            "Staff DevOps Engineer, DevInfra",
            Some("Wiraa"),
            None,
            Some("linkedin"),
            None,
            None,
            None,
        )?;

        // Check for duplicate with shorter title (substring)
        let duplicate = db.is_duplicate_job("Staff DevOps Engineer", Some("Wiraa"), None)?;
        assert!(
            duplicate.is_some(),
            "Substring match should be detected as duplicate"
        );

        Ok(())
    }

    #[test]
    fn test_different_employers_not_duplicate() -> Result<()> {
        let db = create_test_db()?;

        // Add job at Company A
        db.add_job_full(
            "DevOps Engineer",
            Some("Company A"),
            None,
            Some("linkedin"),
            None,
            None,
            None,
        )?;

        // Check for duplicate at Company B
        let duplicate = db.is_duplicate_job("DevOps Engineer", Some("Company B"), None)?;
        assert!(
            duplicate.is_none(),
            "Same title at different companies should not be duplicate"
        );

        Ok(())
    }

    #[test]
    fn test_fuzzy_match_same_employer() -> Result<()> {
        let db = create_test_db()?;

        // Add job
        db.add_job_full(
            "Senior Software Engineer",
            Some("Acme Corp"),
            None,
            Some("linkedin"),
            None,
            None,
            None,
        )?;

        // Check for duplicate with very similar title
        let duplicate = db.is_duplicate_job(
            "Sr. Software Engineer",
            Some("Acme Corp"),
            None,
        )?;
        assert!(
            duplicate.is_some(),
            "Fuzzy match should detect similar titles"
        );

        Ok(())
    }

    #[test]
    fn test_url_match_overrides_title() -> Result<()> {
        let db = create_test_db()?;

        // Add job with URL
        db.add_job_full(
            "Job Title A",
            Some("Company A"),
            Some("https://example.com/job/123"),
            Some("linkedin"),
            None,
            None,
            None,
        )?;

        // Check for duplicate with same URL but different title
        let duplicate = db.is_duplicate_job(
            "Job Title B",
            Some("Company B"),
            Some("https://example.com/job/123"),
        )?;
        assert!(
            duplicate.is_some(),
            "URL match should detect duplicate even with different title"
        );

        Ok(())
    }

    #[test]
    fn test_case_insensitive_matching() -> Result<()> {
        let db = create_test_db()?;

        // Add job
        db.add_job_full(
            "DevOps Engineer",
            Some("Wiraa"),
            None,
            Some("linkedin"),
            None,
            None,
            None,
        )?;

        // Check for duplicate with different case
        let duplicate = db.is_duplicate_job("devops engineer", Some("WIRAA"), None)?;
        assert!(
            duplicate.is_some(),
            "Matching should be case-insensitive"
        );

        Ok(())
    }

    #[test]
    fn test_find_duplicates() -> Result<()> {
        let db = create_test_db()?;

        // Add original job
        db.add_job_full(
            "DevOps Engineer",
            Some("Wiraa"),
            None,
            Some("linkedin"),
            None,
            None,
            None,
        )?;

        // Add duplicate
        db.add_job_full(
            "DevOps Engineer",
            Some("Wiraa"),
            None,
            Some("indeed"),
            None,
            None,
            None,
        )?;

        // Add another job at different company (not duplicate)
        db.add_job_full(
            "DevOps Engineer",
            Some("Other Company"),
            None,
            Some("linkedin"),
            None,
            None,
            None,
        )?;

        let duplicates = db.find_duplicates()?;
        assert_eq!(duplicates.len(), 1, "Should find exactly one duplicate");

        Ok(())
    }
}
