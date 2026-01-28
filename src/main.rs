mod db;
mod models;

use anyhow::Result;
use clap::{Parser, Subcommand};
use db::Database;

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
    }

    Ok(())
}

fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        format!("{}...", &s[..max.saturating_sub(3)])
    }
}
