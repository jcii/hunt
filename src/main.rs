mod db;
mod email;
mod models;

use anyhow::{anyhow, Context, Result};
use clap::{Parser, Subcommand};
use db::Database;
use email::{EmailConfig, EmailIngester};
use models::{BaseResume, Job};
use std::path::PathBuf;

#[derive(Parser)]
#[command(name = "hunt")]
#[command(about = "Job search automation - find, track, and analyze opportunities")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Initialize the database
    Init,

    /// Add a job posting
    Add {
        /// URL or text of job posting
        content: String,
    },

    /// List jobs
    List {
        /// Filter by status (new, reviewing, applied, rejected, closed)
        #[arg(short, long)]
        status: Option<String>,

        /// Filter by employer
        #[arg(short, long)]
        employer: Option<String>,
    },

    /// Show job details
    Show {
        /// Job ID
        id: i64,
    },

    /// Manage employers
    Employer {
        #[command(subcommand)]
        command: EmployerCommands,
    },

    /// Show ranked jobs
    Rank {
        /// Number of jobs to show
        #[arg(short, long, default_value = "10")]
        limit: usize,
    },

    /// Fetch job alerts from email
    Email {
        /// Gmail address
        #[arg(short, long, default_value = "jciispam@gmail.com")]
        username: String,

        /// Path to app password file
        #[arg(short, long, default_value = "~/.gmail.app_password.txt")]
        password_file: String,

        /// Number of days to look back
        #[arg(short, long, default_value = "7")]
        days: u32,

        /// Dry run - show what would be added without adding
        #[arg(long)]
        dry_run: bool,
    },

    /// Manage resumes
    Resume {
        #[command(subcommand)]
        command: ResumeCommands,
    },

    /// Clean up bad data in the database
    Cleanup {
        /// Remove navigation artifacts (non-job titles)
        #[arg(long)]
        artifacts: bool,

        /// Remove duplicate jobs (keep first)
        #[arg(long)]
        duplicates: bool,

        /// Run all cleanup operations
        #[arg(long)]
        all: bool,

        /// Show what would be removed without removing
        #[arg(long)]
        dry_run: bool,
    },
}

#[derive(Subcommand)]
enum EmployerCommands {
    /// List all employers
    List {
        /// Filter by status (ok, yuck, never)
        #[arg(short, long)]
        status: Option<String>,
    },

    /// Mark employer as blocked (never apply)
    Block {
        /// Employer name
        name: String,
    },

    /// Mark employer as undesirable (apply reluctantly)
    Yuck {
        /// Employer name
        name: String,
    },

    /// Clear employer status (ok to apply)
    Ok {
        /// Employer name
        name: String,
    },

    /// Show employer details
    Show {
        /// Employer name or ID
        name: String,
    },
}

#[derive(Subcommand)]
enum ResumeCommands {
    /// Add a base resume
    Add {
        /// Name for this resume
        name: String,

        /// Format (markdown, plain, json, latex)
        #[arg(short, long, default_value = "markdown")]
        format: String,

        /// Path to resume file
        file: PathBuf,

        /// Optional notes about this resume
        #[arg(short, long)]
        notes: Option<String>,
    },

    /// List base resumes
    List,

    /// Show a base resume
    Show {
        /// Resume name or ID
        name: String,
    },

    /// Generate a tailored resume variant for a job
    Tailor {
        /// Job ID to tailor resume for
        job_id: i64,

        /// Base resume name or ID
        #[arg(short, long)]
        resume: String,

        /// Output file path
        #[arg(short, long)]
        output: Option<PathBuf>,
    },

    /// List resume variants for a job
    Variants {
        /// Job ID
        job_id: i64,
    },
}

fn cleanup_artifacts(db: &Database, dry_run: bool) -> Result<usize> {
    // Patterns that indicate navigation artifacts
    let artifact_patterns = [
        "view this job",
        "view job",
        "apply now",
        "see more",
        "view all",
        "click here",
        "learn more",
        "read more",
        "get started",
        "sign in",
        "log in",
        "unsubscribe",
    ];

    let jobs = db.list_jobs(None, None)?;
    let mut removed = 0;

    for job in jobs {
        let title_lower = job.title.to_lowercase();

        // Check if title is too short (likely not a real job)
        if job.title.len() < 5 {
            if !dry_run {
                db.delete_job(job.id)?;
            }
            removed += 1;
            continue;
        }

        // Check if title matches artifact patterns
        let is_artifact = artifact_patterns.iter().any(|pattern| {
            title_lower.contains(pattern) && title_lower.len() < 50
        });

        if is_artifact {
            if !dry_run {
                db.delete_job(job.id)?;
            }
            removed += 1;
        }
    }

    Ok(removed)
}

fn cleanup_duplicates(db: &Database, dry_run: bool) -> Result<usize> {
    let jobs = db.list_jobs(None, None)?;
    let mut seen: std::collections::HashMap<String, i64> = std::collections::HashMap::new();
    let mut removed = 0;

    // Group by (title, employer) - keep the first one (lowest ID)
    for job in jobs {
        let employer = job.employer_name.as_deref().unwrap_or("");
        let key = format!("{}|||{}", job.title.to_lowercase(), employer.to_lowercase());

        if seen.contains_key(&key) {
            // This is a duplicate - remove it
            if !dry_run {
                db.delete_job(job.id)?;
            }
            removed += 1;
        } else {
            // First occurrence - remember it
            seen.insert(key, job.id);
        }
    }

    Ok(removed)
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let db = Database::open()?;

    match cli.command {
        Commands::Init => {
            db.init()?;
            println!("Database initialized at {}", db.path().display());
        }

        Commands::Add { content } => {
            db.ensure_initialized()?;
            let job_id = db.add_job(&content)?;
            println!("Added job #{}", job_id);
        }

        Commands::List { status, employer } => {
            db.ensure_initialized()?;
            let jobs = db.list_jobs(status.as_deref(), employer.as_deref())?;
            if jobs.is_empty() {
                println!("No jobs found.");
            } else {
                println!("{:<6} {:<12} {:<30} {:<20} {:>12}", "ID", "STATUS", "TITLE", "EMPLOYER", "PAY RANGE");
                println!("{}", "-".repeat(84));
                for job in jobs {
                    let pay = match (job.pay_min, job.pay_max) {
                        (Some(min), Some(max)) => format!("${}-${}", min / 1000, max / 1000),
                        (Some(min), None) => format!("${}+", min / 1000),
                        (None, Some(max)) => format!("<${}", max / 1000),
                        (None, None) => "-".to_string(),
                    };
                    println!(
                        "{:<6} {:<12} {:<30} {:<20} {:>12}",
                        job.id,
                        job.status,
                        truncate(&job.title, 28),
                        truncate(&job.employer_name.unwrap_or_default(), 18),
                        pay
                    );
                }
            }
        }

        Commands::Show { id } => {
            db.ensure_initialized()?;
            match db.get_job(id)? {
                Some(job) => {
                    println!("Job #{}", job.id);
                    println!("Title: {}", job.title);
                    if let Some(employer) = &job.employer_name {
                        println!("Employer: {}", employer);
                    }
                    println!("Status: {}", job.status);
                    if let Some(url) = &job.url {
                        println!("URL: {}", url);
                    }
                    if let Some(source) = &job.source {
                        println!("Source: {}", source);
                    }
                    match (job.pay_min, job.pay_max) {
                        (Some(min), Some(max)) => println!("Pay: ${} - ${}", min, max),
                        (Some(min), None) => println!("Pay: ${}+", min),
                        (None, Some(max)) => println!("Pay: up to ${}", max),
                        (None, None) => {}
                    }
                    println!("Created: {}", job.created_at);
                    if let Some(raw) = &job.raw_text {
                        println!("\n--- Raw Text ---\n{}", raw);
                    }
                }
                None => {
                    println!("Job #{} not found.", id);
                }
            }
        }

        Commands::Employer { command } => {
            db.ensure_initialized()?;
            match command {
                EmployerCommands::List { status } => {
                    let employers = db.list_employers(status.as_deref())?;
                    if employers.is_empty() {
                        println!("No employers found.");
                    } else {
                        println!("{:<6} {:<8} {:<30} {:<30}", "ID", "STATUS", "NAME", "DOMAIN");
                        println!("{}", "-".repeat(76));
                        for emp in employers {
                            println!(
                                "{:<6} {:<8} {:<30} {:<30}",
                                emp.id,
                                emp.status,
                                truncate(&emp.name, 28),
                                truncate(&emp.domain.unwrap_or_default(), 28)
                            );
                        }
                    }
                }

                EmployerCommands::Block { name } => {
                    db.set_employer_status(&name, "never")?;
                    println!("Marked '{}' as NEVER (blocked).", name);
                }

                EmployerCommands::Yuck { name } => {
                    db.set_employer_status(&name, "yuck")?;
                    println!("Marked '{}' as YUCK (undesirable).", name);
                }

                EmployerCommands::Ok { name } => {
                    db.set_employer_status(&name, "ok")?;
                    println!("Marked '{}' as OK.", name);
                }

                EmployerCommands::Show { name } => {
                    match db.get_employer_by_name(&name)? {
                        Some(emp) => {
                            println!("Employer #{}", emp.id);
                            println!("Name: {}", emp.name);
                            println!("Status: {}", emp.status);
                            if let Some(domain) = &emp.domain {
                                println!("Domain: {}", domain);
                            }
                            if let Some(notes) = &emp.notes {
                                println!("Notes: {}", notes);
                            }
                            let jobs = db.list_jobs(None, Some(&emp.name))?;
                            if !jobs.is_empty() {
                                println!("\nJobs ({}):", jobs.len());
                                for job in jobs {
                                    println!("  #{} - {} ({})", job.id, job.title, job.status);
                                }
                            }
                        }
                        None => {
                            println!("Employer '{}' not found.", name);
                        }
                    }
                }
            }
        }

        Commands::Rank { limit } => {
            db.ensure_initialized()?;
            let jobs = db.rank_jobs(limit)?;
            if jobs.is_empty() {
                println!("No jobs to rank.");
            } else {
                println!("{:<5} {:<6} {:<12} {:<25} {:<18} {:>10}", "RANK", "ID", "STATUS", "TITLE", "EMPLOYER", "SCORE");
                println!("{}", "-".repeat(80));
                for (i, (job, score)) in jobs.iter().enumerate() {
                    println!(
                        "{:<5} {:<6} {:<12} {:<25} {:<18} {:>10.1}",
                        i + 1,
                        job.id,
                        job.status,
                        truncate(&job.title, 23),
                        truncate(&job.employer_name.clone().unwrap_or_default(), 16),
                        score
                    );
                }
            }
        }

        Commands::Email {
            username,
            password_file,
            days,
            dry_run,
        } => {
            db.ensure_initialized()?;

            // Expand ~ in path
            let password_path = if password_file.starts_with("~/") {
                let home = std::env::var("HOME").unwrap_or_default();
                PathBuf::from(format!("{}/{}", home, &password_file[2..]))
            } else {
                PathBuf::from(&password_file)
            };

            println!("Connecting to Gmail as {}...", username);
            let config = EmailConfig::from_gmail_password_file(&username, &password_path)?;
            let ingester = EmailIngester::new(config);

            println!("Searching for job alerts from the last {} days...", days);
            let stats = ingester.fetch_job_alerts(&db, days, dry_run)?;

            println!("\nResults:");
            println!("  Emails found: {}", stats.emails_found);
            println!("  Jobs added:   {}", stats.jobs_added);
            if stats.errors > 0 {
                println!("  Errors:       {}", stats.errors);
            }

            if dry_run {
                println!("\n(Dry run - no jobs were actually added)");
            }
        }

        Commands::Resume { command } => {
            db.ensure_initialized()?;
            match command {
                ResumeCommands::Add {
                    name,
                    format,
                    file,
                    notes,
                } => {
                    let content = std::fs::read_to_string(&file)
                        .with_context(|| format!("Failed to read resume file: {}", file.display()))?;

                    let resume_id = db.create_base_resume(&name, &format, &content, notes.as_deref())?;
                    println!("Added base resume '{}' (ID: {})", name, resume_id);
                }

                ResumeCommands::List => {
                    let resumes = db.list_base_resumes()?;
                    if resumes.is_empty() {
                        println!("No base resumes found.");
                    } else {
                        println!("{:<6} {:<20} {:<10} {:<20}", "ID", "NAME", "FORMAT", "UPDATED");
                        println!("{}", "-".repeat(58));
                        for resume in resumes {
                            println!(
                                "{:<6} {:<20} {:<10} {:<20}",
                                resume.id,
                                truncate(&resume.name, 18),
                                resume.format,
                                truncate(&resume.updated_at, 18)
                            );
                        }
                    }
                }

                ResumeCommands::Show { name } => {
                    let resume = if let Ok(id) = name.parse::<i64>() {
                        db.get_base_resume(id)?
                    } else {
                        db.get_base_resume_by_name(&name)?
                    };

                    match resume {
                        Some(resume) => {
                            println!("Resume '{}' (ID: {})", resume.name, resume.id);
                            println!("Format: {}", resume.format);
                            if let Some(notes) = &resume.notes {
                                println!("Notes: {}", notes);
                            }
                            println!("Created: {}", resume.created_at);
                            println!("Updated: {}", resume.updated_at);
                            println!("\n--- Content ---\n{}", resume.content);
                        }
                        None => {
                            println!("Resume '{}' not found.", name);
                        }
                    }
                }

                ResumeCommands::Tailor {
                    job_id,
                    resume,
                    output,
                } => {
                    let job = db.get_job(job_id)?
                        .ok_or_else(|| anyhow!("Job #{} not found", job_id))?;

                    let base_resume = if let Ok(id) = resume.parse::<i64>() {
                        db.get_base_resume(id)?
                    } else {
                        db.get_base_resume_by_name(&resume)?
                    }
                    .ok_or_else(|| anyhow!("Resume '{}' not found", resume))?;

                    let tailored_content = tailor_resume_for_job(&base_resume, &job)?;
                    let notes = format!("Tailored for: {}", job.title);

                    let variant_id = db.create_resume_variant(
                        base_resume.id,
                        job_id,
                        &tailored_content,
                        Some(&notes),
                    )?;

                    if let Some(out_path) = output {
                        std::fs::write(&out_path, &tailored_content)
                            .with_context(|| format!("Failed to write to {}", out_path.display()))?;
                        println!("Tailored resume saved to: {}", out_path.display());
                    } else {
                        println!("Tailored resume for job #{} (variant ID: {})", job_id, variant_id);
                        println!("\n--- Tailored Resume ---\n{}", tailored_content);
                    }
                }

                ResumeCommands::Variants { job_id } => {
                    let variants = db.list_resume_variants_for_job(job_id)?;
                    if variants.is_empty() {
                        println!("No resume variants found for job #{}.", job_id);
                    } else {
                        println!("{:<6} {:<15} {:<20}", "ID", "BASE RESUME", "CREATED");
                        println!("{}", "-".repeat(43));
                        for variant in variants {
                            let base_resume = db.get_base_resume(variant.base_resume_id)?
                                .ok_or_else(|| anyhow!("Base resume not found"))?;
                            println!(
                                "{:<6} {:<15} {:<20}",
                                variant.id,
                                truncate(&base_resume.name, 13),
                                truncate(&variant.created_at, 18)
                            );
                        }
                    }
                }
            }
        }

        Commands::Cleanup {
            artifacts,
            duplicates,
            all,
            dry_run,
        } => {
            db.ensure_initialized()?;

            let mut total_removed = 0;

            if artifacts || all {
                println!("Checking for navigation artifacts...");
                let removed = cleanup_artifacts(&db, dry_run)?;
                total_removed += removed;
                if dry_run {
                    println!("  Would remove {} artifact(s)", removed);
                } else {
                    println!("  Removed {} artifact(s)", removed);
                }
            }

            if duplicates || all {
                println!("Checking for duplicate jobs...");
                let removed = cleanup_duplicates(&db, dry_run)?;
                total_removed += removed;
                if dry_run {
                    println!("  Would remove {} duplicate(s)", removed);
                } else {
                    println!("  Removed {} duplicate(s)", removed);
                }
            }

            if !artifacts && !duplicates && !all {
                println!("No cleanup operation specified. Use --artifacts, --duplicates, or --all");
            } else if dry_run {
                println!("\nTotal that would be removed: {}", total_removed);
            } else {
                println!("\nTotal removed: {}", total_removed);
            }
        }
    }

    Ok(())
}

fn tailor_resume_for_job(base_resume: &BaseResume, job: &Job) -> Result<String> {
    let mut tailored = String::new();

    tailored.push_str(&format!("# Resume - Tailored for: {}\n\n", job.title));

    if let Some(employer) = &job.employer_name {
        tailored.push_str(&format!("**Position**: {} at {}\n\n", job.title, employer));
    } else {
        tailored.push_str(&format!("**Position**: {}\n\n", job.title));
    }

    if job.pay_min.is_some() || job.pay_max.is_some() {
        let pay_range = match (job.pay_min, job.pay_max) {
            (Some(min), Some(max)) => format!("${} - ${}", min, max),
            (Some(min), None) => format!("${}+", min),
            (None, Some(max)) => format!("up to ${}", max),
            (None, None) => "Not specified".to_string(),
        };
        tailored.push_str(&format!("**Compensation**: {}\n\n", pay_range));
    }

    tailored.push_str("---\n\n");
    tailored.push_str(&base_resume.content);

    tailored.push_str("\n\n---\n");
    tailored.push_str(&format!("\n*Tailored from base resume: {}*\n", base_resume.name));
    tailored.push_str(&format!("*Generated: {}*\n", chrono::Local::now().format("%Y-%m-%d %H:%M:%S")));

    Ok(tailored)
}

fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        format!("{}...", &s[..max.saturating_sub(3)])
    }
}
