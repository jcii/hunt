mod ai;
mod browser;
mod db;
mod email;
mod models;
mod tui;

use anyhow::{anyhow, Context, Result};
use clap::{Parser, Subcommand};
use db::Database;
use email::{EmailConfig, EmailIngester};
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

        /// Show raw job description text even when AI summary exists
        #[arg(long)]
        raw: bool,
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

    /// Track Glassdoor reviews for watched employers
    Glassdoor {
        #[command(subcommand)]
        command: GlassdoorCommands,
    },

    /// Destroy all data in the database
    Destroy {
        /// Actually execute the wipe (required for safety)
        #[arg(long)]
        confirm: bool,
    },

    /// Research startups
    Startup {
        #[command(subcommand)]
        command: StartupCommands,
    },

    /// Fetch job description from URL
    Fetch {
        /// Job ID to fetch (not used with --all)
        #[arg(required_unless_present = "all")]
        id: Option<i64>,

        /// Fetch all jobs without descriptions
        #[arg(long)]
        all: bool,

        /// Re-fetch jobs even if they already have descriptions (used with --all)
        #[arg(long)]
        force: bool,

        /// Maximum number of jobs to fetch (used with --all)
        #[arg(long)]
        limit: Option<usize>,

        /// Seconds to wait between fetches (default: 5)
        #[arg(long, default_value_t = 5)]
        delay: u64,

        /// Run browser in headless mode (may not work with LinkedIn auth)
        #[arg(long)]
        headless: bool,
    },

    /// AI-powered job analysis
    Analyze {
        /// Job ID to analyze
        job_id: i64,

        /// AI model to use (default: claude-sonnet)
        #[arg(short, long, default_value = "claude-sonnet")]
        model: String,
    },

    /// Extract keywords from a job posting
    Keywords {
        /// Job ID to extract keywords from
        #[arg(required_unless_present_any = ["search", "all"])]
        job_id: Option<i64>,

        /// AI model to use (default: claude-sonnet)
        #[arg(short, long, default_value = "claude-sonnet")]
        model: String,

        /// Search for a keyword across all jobs
        #[arg(short, long)]
        search: Option<String>,

        /// Show stored keywords without re-running AI
        #[arg(long)]
        show: bool,

        /// Extract keywords from all jobs with descriptions but no stored keywords
        #[arg(long)]
        all: bool,

        /// Re-extract keywords even if they already exist (use with --all)
        #[arg(long)]
        force: bool,
    },

    /// Analyze resume fit against a job posting
    Fit {
        /// Job ID to compare against
        job_id: i64,

        /// Base resume name or ID
        #[arg(short, long)]
        resume: String,

        /// AI model to use (default: claude-sonnet)
        #[arg(short, long, default_value = "claude-sonnet")]
        model: String,
    },

    /// Browse jobs interactively in a TUI
    Browse {
        /// Filter by status (new, reviewing, applied, rejected, closed)
        #[arg(short, long)]
        status: Option<String>,

        /// Filter by employer
        #[arg(short, long)]
        employer: Option<String>,
    },

    /// Run full refresh pipeline: email → fetch → keywords
    Refresh {
        /// Gmail address
        #[arg(short, long, default_value = "jciispam@gmail.com")]
        username: String,

        /// Path to app password file
        #[arg(short, long, default_value = "~/.gmail.app_password.txt")]
        password_file: String,

        /// Number of days to look back for emails
        #[arg(short, long, default_value = "7")]
        days: u32,

        /// AI model for keyword extraction
        #[arg(short, long, default_value = "claude-sonnet")]
        model: String,

        /// Run browser in headless mode
        #[arg(long)]
        headless: bool,

        /// Seconds to wait between fetches
        #[arg(long, default_value_t = 5)]
        delay: u64,
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

    /// Research startup info (funding, YC, HN mentions)
    Research {
        /// Employer name
        name: String,
    },

    /// Research public company controversies and practices
    Evil {
        /// Employer name
        name: String,
    },

    /// Research private company ownership (parent, PE/VC, investors)
    Ownership {
        /// Employer name
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

        /// Single AI model to use (default: claude-sonnet)
        #[arg(long, default_value = "claude-sonnet")]
        model: String,

        /// Multiple AI models (comma-separated, e.g. claude-sonnet,gpt-4o)
        #[arg(long)]
        models: Option<String>,

        /// Output format: markdown or latex (default: markdown)
        #[arg(short, long, default_value = "markdown")]
        format: String,

        /// Output file path
        #[arg(short, long)]
        output: Option<PathBuf>,
    },

    /// List resume variants for a job
    Variants {
        /// Job ID
        job_id: i64,
    },

    /// Compare resume variants for a job side by side
    Compare {
        /// Job ID
        job_id: i64,
    },
}

#[derive(Subcommand)]
enum GlassdoorCommands {
    /// Fetch reviews for employers via AI research
    Fetch {
        /// Specific employer name
        #[arg(short, long)]
        employer: Option<String>,

        /// Fetch for all employers (not just 'ok' status)
        #[arg(long)]
        all: bool,

        /// Re-fetch even if reviews already exist
        #[arg(long)]
        force: bool,

        /// AI model to use
        #[arg(short, long, default_value = "claude-sonnet")]
        model: String,

        /// Dry run - show what would be fetched without storing
        #[arg(long)]
        dry_run: bool,
    },

    /// List all employers with Glassdoor data
    List,

    /// Show Glassdoor reviews and summary for an employer
    Show {
        /// Employer name
        employer: String,
    },
}

#[derive(Subcommand)]
enum StartupCommands {
    /// Research startup information for an employer
    Research {
        /// Employer name
        employer: String,
    },
}

// (glassdoor reviews now fetched via AI in ai::research_glassdoor)

#[derive(Debug, Default)]
struct StartupResearchData {
    crunchbase_url: Option<String>,
    funding_stage: Option<String>,
    total_funding: Option<i64>,
    last_funding_date: Option<String>,
    yc_batch: Option<String>,
    yc_url: Option<String>,
    hn_mentions_count: Option<i64>,
    recent_news: Option<String>,
}

#[derive(Debug, Default)]
struct PublicCompanyResearchData {
    controversies: Option<String>,
    labor_practices: Option<String>,
    environmental_issues: Option<String>,
    political_donations: Option<String>,
    evil_summary: Option<String>,
}

fn research_startup(name: &str) -> Result<StartupResearchData> {
    let mut data = StartupResearchData::default();

    // Research YC companies
    if let Ok(yc_info) = search_yc_company(name) {
        data.yc_batch = yc_info.batch;
        data.yc_url = yc_info.url;
    }

    // Research HN mentions
    if let Ok(hn_count) = search_hn_mentions(name) {
        data.hn_mentions_count = Some(hn_count);
    }

    // Note: Crunchbase requires API access or scraping, which is more complex
    // For now, we'll leave this as a placeholder for future implementation
    // data.crunchbase_url = search_crunchbase(name)?;

    Ok(data)
}

#[derive(Debug)]
struct YCCompanyInfo {
    batch: Option<String>,
    url: Option<String>,
}

fn search_yc_company(_name: &str) -> Result<YCCompanyInfo> {
    // YC has a companies list at https://www.ycombinator.com/companies
    // For now, this is a stub implementation that could be enhanced with actual API/scraping
    // TODO: Implement actual YC company search
    Ok(YCCompanyInfo {
        batch: None,
        url: None,
    })
}

fn search_hn_mentions(_name: &str) -> Result<i64> {
    // Use HN Algolia API to search for mentions
    // https://hn.algolia.com/api
    // For now, this is a stub implementation
    // TODO: Implement actual HN search via Algolia API
    Ok(0)
}

fn research_public_company(name: &str) -> Result<PublicCompanyResearchData> {
    let mut data = PublicCompanyResearchData::default();

    // Note: This is a placeholder implementation
    // In a real implementation, you would:
    // 1. Search for news articles about controversies
    // 2. Look up labor practice reports and ratings
    // 3. Check environmental/ESG scores from sources like CDP, EPA
    // 4. Research political donations via OpenSecrets or FEC data
    // 5. Compile a summary with sources

    // For now, return a placeholder that indicates research capability exists
    data.evil_summary = Some(format!(
        "Research framework ready for {}. Implementation pending: \
         controversies tracking, labor practice ratings, environmental scores, \
         political donation analysis. Sources to integrate: news APIs, OpenSecrets, \
         EPA/CDP data, labor watch organizations.",
        name
    ));

    Ok(data)
}

#[derive(Debug, Default)]
struct PrivateOwnershipData {
    parent_company: Option<String>,
    pe_owner: Option<String>,
    pe_firm_url: Option<String>,
    vc_investors: Option<String>,
    key_investors: Option<String>,
    ownership_concerns: Option<String>,
    ownership_type: Option<String>,
}

fn research_private_ownership(_name: &str) -> Result<PrivateOwnershipData> {
    let mut data = PrivateOwnershipData::default();

    // Research parent company
    if let Ok(parent_info) = search_parent_company(_name) {
        data.parent_company = parent_info.parent_name;
        data.ownership_type = Some(parent_info.relationship_type);
    }

    // Research PE/VC ownership
    if let Ok(pe_info) = search_pe_ownership(_name) {
        data.pe_owner = pe_info.firm_name;
        data.pe_firm_url = pe_info.firm_url;
    }

    // Research investor information
    if let Ok(investors) = search_investor_info(_name) {
        if !investors.is_empty() {
            data.vc_investors = Some(investors.join(", "));
        }
    }

    // Check for ownership concerns
    if let Ok(concerns) = search_ownership_concerns(_name) {
        if !concerns.is_empty() {
            data.ownership_concerns = Some(concerns.join("; "));
        }
    }

    Ok(data)
}

#[derive(Debug)]
struct ParentCompanyInfo {
    parent_name: Option<String>,
    relationship_type: String,
}

fn search_parent_company(_name: &str) -> Result<ParentCompanyInfo> {
    // TODO: Implement parent company research via:
    // - Crunchbase API
    // - LinkedIn company pages
    // - SEC EDGAR filings for public companies
    // - PitchBook data
    Ok(ParentCompanyInfo {
        parent_name: None,
        relationship_type: "independent".to_string(),
    })
}

#[derive(Debug)]
struct PEOwnershipInfo {
    firm_name: Option<String>,
    firm_url: Option<String>,
}

fn search_pe_ownership(_name: &str) -> Result<PEOwnershipInfo> {
    // TODO: Implement PE/VC ownership research via:
    // - Crunchbase API for funding rounds
    // - PitchBook for PE ownership
    // - Company press releases
    // - LinkedIn company pages
    Ok(PEOwnershipInfo {
        firm_name: None,
        firm_url: None,
    })
}

fn search_investor_info(_name: &str) -> Result<Vec<String>> {
    // TODO: Implement investor research via:
    // - Crunchbase API for investor lists
    // - PitchBook data
    // - Company announcements
    // - SEC filings for public investors
    Ok(vec![])
}

fn search_ownership_concerns(_name: &str) -> Result<Vec<String>> {
    // TODO: Implement concern detection via:
    // - News articles about controversial owners
    // - ESG databases
    // - Regulatory filings
    // - Public controversy tracking
    Ok(vec![])
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

        // Check if URL is a non-job link (alerts, search, settings, etc.)
        let is_non_job_url = job.url.as_ref().is_some_and(|url| {
            email::is_search_link(url)
        });

        if is_artifact || is_non_job_url {
            if !dry_run {
                db.delete_job(job.id)?;
            }
            removed += 1;
        }
    }

    Ok(removed)
}

fn cleanup_duplicates(db: &Database, dry_run: bool) -> Result<usize> {
    // Use sophisticated duplicate detection that handles:
    // - Exact matches (case-insensitive)
    // - Substring matches
    // - Fuzzy matching (>80% similar via Jaro-Winkler)
    // - URL-based deduplication
    let duplicates = db.find_duplicates()?;

    if !dry_run {
        for (_, duplicate_id, _) in &duplicates {
            db.delete_job(*duplicate_id)?;
        }
    }

    Ok(duplicates.len())
}

fn display_domain_keywords(keywords: &[models::JobKeyword]) {
    // Legend
    println!("  *** = required   ** = important   * = nice-to-have\n");

    let domains = [
        ("tech", "TECH"),
        ("discipline", "DISCIPLINE"),
        ("cloud", "CLOUD"),
        ("soft_skill", "SOFT SKILLS"),
    ];

    for (domain_key, domain_label) in &domains {
        let domain_keywords: Vec<&models::JobKeyword> = keywords
            .iter()
            .filter(|k| k.domain == *domain_key)
            .collect();

        if domain_keywords.is_empty() {
            continue;
        }

        println!("  {}", domain_label);
        for weight in (1..=3).rev() {
            let at_weight: Vec<&str> = domain_keywords
                .iter()
                .filter(|k| k.weight == weight)
                .map(|k| k.keyword.as_str())
                .collect();

            if at_weight.is_empty() {
                continue;
            }

            let stars = "*".repeat(weight as usize);
            let pad = " ".repeat(3 - weight as usize);
            println!("    {}{} {}", pad, stars, at_weight.join(", "));
        }
        println!();
    }
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
                println!("{:<6} {:<10} {:<40} {:<25} {:>15} {:<60}", "ID", "STATUS", "TITLE", "EMPLOYER", "PAY RANGE", "URL");
                println!("{}", "-".repeat(160));
                for job in jobs {
                    let pay = match (job.pay_min, job.pay_max) {
                        (Some(min), Some(max)) => format!("${}-${}", min / 1000, max / 1000),
                        (Some(min), None) => format!("${}+", min / 1000),
                        (None, Some(max)) => format!("<${}", max / 1000),
                        (None, None) => "-".to_string(),
                    };
                    let url = job.url.as_deref().unwrap_or("-");
                    println!(
                        "{:<6} {:<10} {:<40} {:<25} {:>15} {:<60}",
                        job.id,
                        job.status,
                        truncate(&job.title, 38),
                        truncate(&job.employer_name.unwrap_or_default(), 23),
                        pay,
                        truncate(url, 58)
                    );
                }
            }
        }

        Commands::Show { id, raw } => {
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

                    // Show AI keywords/profile if available
                    let has_ai = if let Some(model) = db.get_latest_keyword_model(id)? {
                        let keywords = db.get_job_keywords(id, Some(&model))?;
                        if !keywords.is_empty() {
                            println!("\n--- Keywords (model: {}) ---\n", model);
                            display_domain_keywords(&keywords);
                            if let Some(profile) = db.get_keyword_profile(id)? {
                                println!("  PROFILE");
                                for line in textwrap::fill(&profile.profile, 72).lines() {
                                    println!("  {}", line);
                                }
                                println!();
                            }
                            true
                        } else {
                            false
                        }
                    } else {
                        false
                    };

                    // Show raw text: always if --raw, or if no AI data exists
                    if raw || !has_ai {
                        if let Some(text) = &job.raw_text {
                            println!("--- Raw Text ---\n{}", text);
                        }
                    } else if job.raw_text.is_some() {
                        println!("(Raw text available — use --raw to display)");
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

                            // Show startup research data if available
                            if emp.yc_batch.is_some() || emp.funding_stage.is_some() || emp.hn_mentions_count.is_some() {
                                println!("\n--- Startup Research ---");
                                if let Some(batch) = &emp.yc_batch {
                                    println!("YC Batch: {}", batch);
                                    if let Some(url) = &emp.yc_url {
                                        println!("YC URL: {}", url);
                                    }
                                }
                                if let Some(stage) = &emp.funding_stage {
                                    println!("Funding Stage: {}", stage);
                                }
                                if let Some(funding) = emp.total_funding {
                                    println!("Total Funding: ${}", funding);
                                }
                                if let Some(date) = &emp.last_funding_date {
                                    println!("Last Funding: {}", date);
                                }
                                if let Some(cb_url) = &emp.crunchbase_url {
                                    println!("Crunchbase: {}", cb_url);
                                }
                                if let Some(count) = emp.hn_mentions_count {
                                    println!("HN Mentions: {}", count);
                                }
                                if let Some(news) = &emp.recent_news {
                                    println!("Recent News: {}", news);
                                }
                                if let Some(updated) = &emp.research_updated_at {
                                    println!("Research Updated: {}", updated);
                                }
                            }

                            // Show public company research data if available
                            if emp.controversies.is_some() || emp.labor_practices.is_some()
                                || emp.environmental_issues.is_some() || emp.political_donations.is_some() {
                                println!("\n--- Public Company Research ---");
                                if let Some(controversies) = &emp.controversies {
                                    println!("Controversies: {}", controversies);
                                }
                                if let Some(labor) = &emp.labor_practices {
                                    println!("Labor Practices: {}", labor);
                                }
                                if let Some(env) = &emp.environmental_issues {
                                    println!("Environmental Issues: {}", env);
                                }
                                if let Some(donations) = &emp.political_donations {
                                    println!("Political Donations: {}", donations);
                                }
                                if let Some(summary) = &emp.evil_summary {
                                    println!("\nEvil Summary:\n{}", summary);
                                }
                                if let Some(updated) = &emp.public_research_updated_at {
                                    println!("Research Updated: {}", updated);
                                }
                            }

                            // Show private ownership research data if available
                            if emp.parent_company.is_some() || emp.pe_owner.is_some() || emp.vc_investors.is_some() {
                                println!("\n--- Ownership Research ---");
                                if let Some(parent) = &emp.parent_company {
                                    println!("Parent Company: {}", parent);
                                }
                                if let Some(ownership_type) = &emp.ownership_type {
                                    println!("Ownership Type: {}", ownership_type);
                                }
                                if let Some(pe) = &emp.pe_owner {
                                    println!("PE Owner: {}", pe);
                                    if let Some(url) = &emp.pe_firm_url {
                                        println!("PE Firm URL: {}", url);
                                    }
                                }
                                if let Some(vc) = &emp.vc_investors {
                                    println!("VC Investors: {}", vc);
                                }
                                if let Some(investors) = &emp.key_investors {
                                    println!("Key Investors: {}", investors);
                                }
                                if let Some(concerns) = &emp.ownership_concerns {
                                    println!("⚠ Concerns: {}", concerns);
                                }
                                if let Some(updated) = &emp.ownership_research_updated {
                                    println!("Ownership Research Updated: {}", updated);
                                }
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

                EmployerCommands::Research { name } => {
                    println!("Researching startup info for '{}'...", name);

                    // Get or create employer
                    let employer_id = db.get_or_create_employer(&name)?;

                    // Perform research
                    let research_data = research_startup(&name)?;

                    // Update database
                    db.update_employer_research(
                        employer_id,
                        research_data.crunchbase_url.as_deref(),
                        research_data.funding_stage.as_deref(),
                        research_data.total_funding,
                        research_data.last_funding_date.as_deref(),
                        research_data.yc_batch.as_deref(),
                        research_data.yc_url.as_deref(),
                        research_data.hn_mentions_count,
                        research_data.recent_news.as_deref(),
                    )?;

                    println!("\n✓ Research complete");
                    if let Some(batch) = &research_data.yc_batch {
                        println!("  YC Batch: {}", batch);
                    }
                    if let Some(stage) = &research_data.funding_stage {
                        println!("  Funding Stage: {}", stage);
                    }
                    if let Some(funding) = research_data.total_funding {
                        println!("  Total Funding: ${}", funding);
                    }
                    if let Some(count) = research_data.hn_mentions_count {
                        println!("  HN Mentions: {}", count);
                    }
                    if let Some(news) = &research_data.recent_news {
                        println!("  Recent News: {}", news);
                    }
                }

                EmployerCommands::Evil { name } => {
                    println!("Researching public company controversies for '{}'...", name);

                    // Get or create employer
                    let employer_id = db.get_or_create_employer(&name)?;

                    // Perform research
                    let research_data = research_public_company(&name)?;

                    // Update database
                    db.update_public_company_research(
                        employer_id,
                        research_data.controversies.as_deref(),
                        research_data.labor_practices.as_deref(),
                        research_data.environmental_issues.as_deref(),
                        research_data.political_donations.as_deref(),
                        research_data.evil_summary.as_deref(),
                    )?;

                    println!("\n✓ Research complete");
                    if let Some(controversies) = &research_data.controversies {
                        println!("  Controversies: {}", controversies);
                    }
                    if let Some(labor) = &research_data.labor_practices {
                        println!("  Labor Practices: {}", labor);
                    }
                    if let Some(env) = &research_data.environmental_issues {
                        println!("  Environmental: {}", env);
                    }
                    if let Some(donations) = &research_data.political_donations {
                        println!("  Political Donations: {}", donations);
                    }
                    if let Some(summary) = &research_data.evil_summary {
                        println!("\n  Summary:\n{}", summary);
                    }
                }

                EmployerCommands::Ownership { name } => {
                    println!("Researching ownership info for '{}'...", name);

                    // Get or create employer
                    let employer_id = db.get_or_create_employer(&name)?;

                    // Perform ownership research
                    let ownership_data = research_private_ownership(&name)?;

                    // Update database
                    db.update_employer_ownership(
                        employer_id,
                        ownership_data.parent_company.as_deref(),
                        ownership_data.pe_owner.as_deref(),
                        ownership_data.pe_firm_url.as_deref(),
                        ownership_data.vc_investors.as_deref(),
                        ownership_data.key_investors.as_deref(),
                        ownership_data.ownership_concerns.as_deref(),
                        ownership_data.ownership_type.as_deref(),
                    )?;

                    println!("\n✓ Ownership research complete");
                    if let Some(parent) = &ownership_data.parent_company {
                        println!("  Parent Company: {}", parent);
                    }
                    if let Some(ownership_type) = &ownership_data.ownership_type {
                        println!("  Ownership Type: {}", ownership_type);
                    }
                    if let Some(pe) = &ownership_data.pe_owner {
                        println!("  PE Owner: {}", pe);
                    }
                    if let Some(vc) = &ownership_data.vc_investors {
                        println!("  VC Investors: {}", vc);
                    }
                    if let Some(investors) = &ownership_data.key_investors {
                        println!("  Key Investors: {}", investors);
                    }
                    if let Some(concerns) = &ownership_data.ownership_concerns {
                        println!("  ⚠ Concerns: {}", concerns);
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
            println!("  Emails processed: {}", stats.emails_found);
            println!("  Jobs added:       {}", stats.jobs_added);
            println!("  Duplicates:       {}", stats.duplicates);
            if stats.errors > 0 {
                println!("  Errors:           {}", stats.errors);
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
                    model,
                    models,
                    format,
                    output,
                } => {
                    let job = db.get_job(job_id)?
                        .ok_or_else(|| anyhow!("Job #{} not found", job_id))?;

                    let job_text = job.raw_text
                        .as_ref()
                        .ok_or_else(|| anyhow!("Job #{} has no raw text for tailoring", job_id))?;

                    let base_resume = if let Ok(id) = resume.parse::<i64>() {
                        db.get_base_resume(id)?
                    } else {
                        db.get_base_resume_by_name(&resume)?
                    }
                    .ok_or_else(|| anyhow!("Resume '{}' not found", resume))?;

                    // Gather all resumes: primary first, then others by updated_at DESC
                    let all_resumes_db = db.list_base_resumes()?;
                    let mut all_resumes: Vec<(String, String)> = Vec::new();
                    // Primary resume first
                    all_resumes.push((base_resume.name.clone(), base_resume.content.clone()));
                    // Other resumes
                    for r in &all_resumes_db {
                        if r.id != base_resume.id {
                            all_resumes.push((r.name.clone(), r.content.clone()));
                        }
                    }

                    // Determine which models to use
                    let model_names: Vec<String> = if let Some(models_str) = &models {
                        models_str.split(',').map(|s| s.trim().to_string()).collect()
                    } else {
                        vec![model.clone()]
                    };

                    let employer_name = job.employer_name.as_deref();

                    for model_name in &model_names {
                        let spec = ai::resolve_model(model_name)?;
                        let provider = ai::create_provider(&spec)?;

                        println!("Generating tailored resume with {} (format: {})...",
                                 spec.short_name, format);

                        let tailored_content = ai::tailor_resume_full(
                            provider.as_ref(),
                            &all_resumes,
                            job_text,
                            &job.title,
                            employer_name,
                            &format,
                        )?;

                        let notes = format!("Tailored for: {} (model: {}, format: {})",
                                           job.title, spec.short_name, format);

                        let variant_id = db.create_resume_variant(
                            base_resume.id,
                            job_id,
                            &tailored_content,
                            Some(&notes),
                            Some(&spec.short_name),
                            Some(&format),
                        )?;

                        if let Some(out_path) = &output {
                            // For multi-model, append model name to filename
                            let final_path = if model_names.len() > 1 {
                                let stem = out_path.file_stem().unwrap_or_default().to_string_lossy();
                                let ext = out_path.extension().map(|e| e.to_string_lossy().to_string())
                                    .unwrap_or_else(|| if format == "latex" { "tex".to_string() } else { "md".to_string() });
                                out_path.with_file_name(format!("{}-{}.{}", stem, spec.short_name, ext))
                            } else {
                                out_path.clone()
                            };
                            std::fs::write(&final_path, &tailored_content)
                                .with_context(|| format!("Failed to write to {}", final_path.display()))?;
                            println!("Saved to: {}", final_path.display());
                        } else {
                            println!("\n--- Tailored Resume (model: {}, variant ID: {}) ---\n{}",
                                     spec.short_name, variant_id, tailored_content);
                        }
                        println!();
                    }
                }

                ResumeCommands::Variants { job_id } => {
                    let variants = db.list_resume_variants_for_job(job_id)?;
                    if variants.is_empty() {
                        println!("No resume variants found for job #{}.", job_id);
                    } else {
                        println!("{:<6} {:<15} {:<15} {:<10} {:<20}", "ID", "BASE RESUME", "MODEL", "FORMAT", "CREATED");
                        println!("{}", "-".repeat(68));
                        for variant in variants {
                            let base_resume = db.get_base_resume(variant.base_resume_id)?
                                .ok_or_else(|| anyhow!("Base resume not found"))?;
                            println!(
                                "{:<6} {:<15} {:<15} {:<10} {:<20}",
                                variant.id,
                                truncate(&base_resume.name, 13),
                                truncate(variant.source_model.as_deref().unwrap_or("-"), 13),
                                variant.output_format.as_deref().unwrap_or("-"),
                                truncate(&variant.created_at, 18)
                            );
                        }
                    }
                }

                ResumeCommands::Compare { job_id } => {
                    let variants = db.list_resume_variants_for_job(job_id)?;
                    if variants.is_empty() {
                        println!("No resume variants found for job #{}.", job_id);
                    } else {
                        let job = db.get_job(job_id)?
                            .ok_or_else(|| anyhow!("Job #{} not found", job_id))?;
                        println!("Resume variants for job #{}: {}\n", job_id, job.title);

                        for variant in &variants {
                            let base_resume = db.get_base_resume(variant.base_resume_id)?
                                .ok_or_else(|| anyhow!("Base resume not found"))?;

                            let model_str = variant.source_model.as_deref().unwrap_or("unknown");
                            let format_str = variant.output_format.as_deref().unwrap_or("unknown");

                            println!("{}", "=".repeat(60));
                            println!("Variant #{} | Base: {} | Model: {} | Format: {}",
                                     variant.id, base_resume.name, model_str, format_str);
                            println!("Created: {}", variant.created_at);
                            println!("{}", "=".repeat(60));
                            println!("{}", variant.content);
                            println!();
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

        Commands::Glassdoor { command } => {
            db.ensure_initialized()?;
            match command {
                GlassdoorCommands::Fetch { employer, all, force, model, dry_run } => {
                    let spec = ai::resolve_model(&model)?;
                    let provider = ai::create_provider(&spec)?;

                    let employers_to_fetch = if let Some(name) = employer {
                        vec![db.get_employer_by_name(&name)?
                            .ok_or_else(|| anyhow!("Employer '{}' not found", name))?]
                    } else if all {
                        db.list_employers(None)?
                    } else {
                        db.list_employers(Some("ok"))?
                    };

                    if employers_to_fetch.is_empty() {
                        println!("No employers found. Use 'hunt employer ok <name>' to watch an employer.");
                        return Ok(());
                    }

                    // Filter out employers that already have reviews (unless --force)
                    let employers_to_fetch: Vec<_> = if force {
                        employers_to_fetch
                    } else {
                        employers_to_fetch.into_iter()
                            .filter(|e| e.glassdoor_review_count.unwrap_or(0) == 0)
                            .collect()
                    };

                    if employers_to_fetch.is_empty() {
                        println!("All employers already have Glassdoor reviews. Use --force to re-fetch.");
                        return Ok(());
                    }

                    println!("Researching Glassdoor reviews for {} employer(s) (model: {}){}...\n",
                             employers_to_fetch.len(), spec.short_name,
                             if force { " --force" } else { "" });
                    let mut total_new = 0;
                    let mut total_errors = 0;

                    for emp in &employers_to_fetch {
                        print!("  {} ... ", emp.name);
                        if dry_run {
                            println!("(dry run)");
                            continue;
                        }

                        match ai::research_glassdoor(provider.as_ref(), &emp.name) {
                            Ok(research) => {
                                let count = research.reviews.len();
                                // Clear old reviews if force
                                if force {
                                    let _ = db.delete_glassdoor_reviews(emp.id);
                                }
                                for review in &research.reviews {
                                    let _ = db.add_glassdoor_review(
                                        emp.id,
                                        review.rating,
                                        Some(&review.title),
                                        Some(&review.pros),
                                        Some(&review.cons),
                                        None,
                                        &review.sentiment,
                                        Some(&review.review_date),
                                    );
                                }
                                let _ = db.update_employer_glassdoor_summary(emp.id);
                                println!("{} reviews", count);
                                total_new += count;
                            }
                            Err(e) => {
                                total_errors += 1;
                                println!("FAILED: {}", e);
                            }
                        }
                    }

                    println!("\n  Added: {}, Errors: {}", total_new, total_errors);
                }

                GlassdoorCommands::List => {
                    let employers = db.list_employers_with_glassdoor()?;
                    if employers.is_empty() {
                        println!("No Glassdoor data collected yet. Run 'hunt glassdoor fetch' to collect.");
                    } else {
                        println!("{:<6} {:<30} {:>6} {:>10} {:<20}",
                                 "ID", "EMPLOYER", "RATING", "REVIEWS", "LAST FETCHED");
                        println!("{}", "-".repeat(75));
                        for emp in &employers {
                            println!("{:<6} {:<30} {:>5.1}★ {:>10} {:<20}",
                                     emp.id,
                                     truncate(&emp.name, 28),
                                     emp.glassdoor_rating.unwrap_or(0.0),
                                     emp.glassdoor_review_count.unwrap_or(0),
                                     emp.last_glassdoor_fetch.as_deref().unwrap_or("-"),
                            );
                        }
                        println!("\nTotal: {} employer(s) with Glassdoor data", employers.len());
                    }
                }

                GlassdoorCommands::Show { employer } => {
                    let emp = db.get_employer_by_name(&employer)?
                        .ok_or_else(|| anyhow!("Employer '{}' not found", employer))?;

                    // Summary
                    let (positive, negative, neutral, avg_rating) = db.get_sentiment_summary(emp.id)?;
                    let total = positive + negative + neutral;

                    if total == 0 {
                        println!("No Glassdoor reviews found for '{}'.", employer);
                        println!("Run 'hunt glassdoor fetch --employer \"{}\"' to collect.", employer);
                        return Ok(());
                    }

                    println!("Glassdoor: {} — {:.1}★ ({} reviews)\n", employer, avg_rating, total);
                    println!("Sentiment:");
                    println!("  Positive: {} ({:.0}%)", positive, positive as f64 / total as f64 * 100.0);
                    println!("  Neutral:  {} ({:.0}%)", neutral, neutral as f64 / total as f64 * 100.0);
                    println!("  Negative: {} ({:.0}%)", negative, negative as f64 / total as f64 * 100.0);

                    if let Some(fetched) = &emp.last_glassdoor_fetch {
                        println!("  Last fetched: {}", fetched);
                    }

                    // Reviews
                    let reviews = db.list_glassdoor_reviews(Some(emp.id))?;
                    if !reviews.is_empty() {
                        println!("\nReviews:\n");
                        for review in reviews {
                            println!("{:<6} {:>4.1}★ {:<10} {}",
                                review.id,
                                review.rating,
                                review.sentiment,
                                review.review_date.as_deref().unwrap_or("-")
                            );
                            if let Some(title) = &review.title {
                                println!("       {}", title);
                            }
                            if let Some(pros) = &review.pros {
                                println!("       Pros: {}", truncate(pros, 60));
                            }
                            if let Some(cons) = &review.cons {
                                println!("       Cons: {}", truncate(cons, 60));
                            }
                            println!();
                        }
                    }
                }
            }
        }

        Commands::Destroy { confirm } => {
            db.ensure_initialized()?;

            // Count what will be destroyed
            let stats = db.get_destruction_stats()?;

            println!("Database destruction preview:");
            println!("  Jobs:               {}", stats.jobs);
            println!("  Job snapshots:      {}", stats.job_snapshots);
            println!("  Employers:          {}", stats.employers);
            println!("  Base resumes:       {}", stats.base_resumes);
            println!("  Resume variants:    {}", stats.resume_variants);
            println!("  Job keywords:       {}", stats.job_keywords);
            println!("  Keyword profiles:   {}", stats.job_keyword_profiles);
            println!("  Fit analyses:       {}", stats.fit_analyses);
            println!("\nTotal records: {}", stats.total());

            if !confirm {
                println!("\n⚠️  This is a preview. To actually destroy all data, run:");
                println!("  hunt destroy --confirm");
            } else {
                println!("\n⚠️  DESTROYING ALL DATA...");
                db.destroy_all_data()?;
                println!("✓ All data destroyed and auto-increment counters reset.");
            }
        }

        Commands::Startup { command } => {
            db.ensure_initialized()?;
            match command {
                StartupCommands::Research { employer } => {
                    println!("Researching startup info for '{}'...", employer);

                    // Get or create employer
                    let employer_id = db.get_or_create_employer(&employer)?;

                    // Perform research
                    let research_data = research_startup(&employer)?;

                    // Update database
                    db.update_employer_research(
                        employer_id,
                        research_data.crunchbase_url.as_deref(),
                        research_data.funding_stage.as_deref(),
                        research_data.total_funding,
                        research_data.last_funding_date.as_deref(),
                        research_data.yc_batch.as_deref(),
                        research_data.yc_url.as_deref(),
                        research_data.hn_mentions_count,
                        research_data.recent_news.as_deref(),
                    )?;

                    println!("\n✓ Research complete");
                    if let Some(batch) = &research_data.yc_batch {
                        println!("  YC Batch: {}", batch);
                    }
                    if let Some(stage) = &research_data.funding_stage {
                        println!("  Funding Stage: {}", stage);
                    }
                    if let Some(funding) = research_data.total_funding {
                        println!("  Total Funding: ${}", funding);
                    }
                    if let Some(count) = research_data.hn_mentions_count {
                        println!("  HN Mentions: {}", count);
                    }
                    if let Some(news) = &research_data.recent_news {
                        println!("  Recent News: {}", news);
                    }
                }
            }
        }

        Commands::Fetch { id, all, force, limit, delay, headless } => {
            db.ensure_initialized()?;

            if all {
                // Fetch all jobs (with or without descriptions based on --force)
                let jobs = db.get_jobs_to_fetch(limit, force)?;

                if jobs.is_empty() {
                    if force {
                        println!("No jobs found!");
                    } else {
                        println!("All jobs have been fetched. Use --force to re-fetch.");
                    }
                    return Ok(());
                }

                let total = jobs.len();
                if force {
                    println!("Found {} jobs to fetch (--force: re-fetching all)", total);
                } else {
                    println!("Found {} unfetched jobs", total);
                }

                // Confirmation prompt for large batches
                if total > 10 {
                    use std::io::{self, Write};
                    print!("Fetch {} jobs? This will take approximately {} minutes. (y/N): ",
                           total, (total as u64 * delay) / 60);
                    io::stdout().flush()?;
                    let mut response = String::new();
                    io::stdin().read_line(&mut response)?;
                    if !response.trim().eq_ignore_ascii_case("y") {
                        println!("Cancelled.");
                        return Ok(());
                    }
                }

                // Warning for short delays
                if delay < 3 {
                    println!("⚠ Warning: Short delay ({} seconds) may trigger rate limiting", delay);
                }

                println!("\nFetching descriptions for {} jobs...\n", total);

                let start_time = std::time::Instant::now();
                let mut success_count = 0;
                let mut fail_count = 0;
                let mut closed_count = 0;
                let mut failed_jobs = Vec::new();

                // Fetch each job
                for (i, job) in jobs.iter().enumerate() {
                    let job_num = i + 1;
                    let employer_name = job.employer_name.as_deref().unwrap_or("Unknown");
                    println!("[{}/{}] Fetching job #{} ({} at {})",
                             job_num, total, job.id,
                             truncate(&job.title, 40),
                             truncate(employer_name, 30));

                    if let Some(url) = &job.url {
                        match fetch_job_description(url, headless) {
                            Ok(job_desc) => {
                                match db.update_job_description(job.id, &job_desc.text,
                                                               job_desc.pay_min, job_desc.pay_max) {
                                    Ok(_) => {
                                        if let Some(ref emp_name) = job_desc.employer_name {
                                            let _ = db.update_job_employer(job.id, emp_name);
                                        }
                                        if job_desc.no_longer_accepting {
                                            let _ = db.update_job_status(job.id, "closed");
                                            println!("⚠ No longer accepting applications — marked as closed");
                                            closed_count += 1;
                                        }
                                        let pay_info = match (job_desc.pay_min, job_desc.pay_max) {
                                            (Some(min), Some(max)) => format!(" | Pay: ${}-${}", min/1000, max/1000),
                                            (Some(min), None) => format!(" | Pay: ${}K+", min/1000),
                                            (None, Some(max)) => format!(" | Pay: up to ${}K", max/1000),
                                            (None, None) => String::new(),
                                        };
                                        println!("✓ Fetched ({} chars{})", job_desc.text.len(), pay_info);
                                        success_count += 1;
                                    }
                                    Err(e) => {
                                        eprintln!("✗ Failed to save: {}", e);
                                        fail_count += 1;
                                        failed_jobs.push((job.id, format!("save error: {}", e)));
                                    }
                                }
                            }
                            Err(e) => {
                                eprintln!("✗ Failed to fetch: {}", e);
                                fail_count += 1;
                                failed_jobs.push((job.id, format!("fetch error: {}", e)));
                            }
                        }
                    } else {
                        eprintln!("✗ No URL available");
                        fail_count += 1;
                        failed_jobs.push((job.id, "no URL".to_string()));
                    }

                    // Delay between fetches (except after last one)
                    if job_num < total {
                        let delay_with_jitter = add_jitter(delay);
                        countdown(delay_with_jitter);
                    }
                }

                // Summary
                let elapsed = start_time.elapsed();
                println!("\n═══════════════════════════════════════════");
                println!("Summary:");
                println!("✓ Successfully fetched: {}/{}", success_count, total);
                if closed_count > 0 {
                    println!("⚠ Closed (no longer accepting): {}", closed_count);
                }
                if fail_count > 0 {
                    println!("✗ Failed: {}/{}", fail_count, total);
                    if !failed_jobs.is_empty() {
                        println!("\nFailed jobs:");
                        for (job_id, reason) in failed_jobs {
                            println!("  Job #{}: {}", job_id, reason);
                        }
                    }
                }
                println!("⏱ Total time: {}m {}s", elapsed.as_secs() / 60, elapsed.as_secs() % 60);
                println!("═══════════════════════════════════════════");

            } else {
                // Single job fetch (original behavior)
                let job_id = id.ok_or_else(|| anyhow!("Job ID required without --all flag"))?;
                let job = db.get_job(job_id)?
                    .ok_or_else(|| anyhow!("Job #{} not found", job_id))?;

                if let Some(url) = &job.url {
                    println!("Fetching job description from: {}", url);
                    if headless {
                        println!("Running in headless mode (may not work with LinkedIn auth)");
                    }

                    // Fetch and extract description
                    let job_desc = fetch_job_description(url, headless)?;

                    // Update job with description and pay info
                    db.update_job_description(job_id, &job_desc.text, job_desc.pay_min, job_desc.pay_max)?;

                    if let Some(ref emp_name) = job_desc.employer_name {
                        db.update_job_employer(job_id, emp_name)?;
                        println!("✓ Employer updated: {}", emp_name);
                    }

                    if job_desc.no_longer_accepting {
                        db.update_job_status(job_id, "closed")?;
                        println!("⚠ Job #{} is no longer accepting applications — marked as closed", job_id);
                    }

                    let pay_info = match (job_desc.pay_min, job_desc.pay_max) {
                        (Some(min), Some(max)) => format!(" | Pay: ${}-${}", min, max),
                        (Some(min), None) => format!(" | Pay: ${}+", min),
                        (None, Some(max)) => format!(" | Pay: up to ${}", max),
                        (None, None) => String::new(),
                    };
                    println!("✓ Job description fetched and stored ({} chars{})", job_desc.text.len(), pay_info);
                } else {
                    println!("Error: Job #{} has no URL", job_id);
                    return Err(anyhow!("Job has no URL to fetch from"));
                }
            }
        }

        Commands::Analyze { job_id, model } => {
            db.ensure_initialized()?;
            let job = db.get_job(job_id)?
                .ok_or_else(|| anyhow!("Job #{} not found", job_id))?;

            let job_text = job.raw_text
                .as_ref()
                .ok_or_else(|| anyhow!("Job #{} has no raw text to analyze", job_id))?;

            let spec = ai::resolve_model(&model)?;
            let provider = ai::create_provider(&spec)?;

            println!("Analyzing job posting #{}: {} (model: {})...\n", job_id, job.title, spec.short_name);

            let analysis = ai::analyze_job(provider.as_ref(), job_text)?;

            println!("=== AI Analysis ===\n");
            println!("{}", analysis);
        }

        Commands::Keywords { job_id, model, search, show, all, force } => {
            db.ensure_initialized()?;

            if let Some(query) = search {
                // Search mode: find keyword across stored job_keywords
                let results = db.search_job_keywords(&query)?;
                if results.is_empty() {
                    println!("No jobs found with keyword matching '{}'.", query);
                } else {
                    println!("Jobs with keyword matching '{}':\n", query);
                    println!("{:<6} {:<14} {:<6} {:<40} {:<30}", "JOB", "DOMAIN", "WT", "TITLE", "KEYWORD");
                    println!("{}", "-".repeat(98));
                    for (job_id, job_title, keyword, domain, weight) in &results {
                        let stars = "*".repeat(*weight as usize);
                        println!(
                            "{:<6} {:<14} {:<6} {:<40} {:<30}",
                            job_id,
                            domain,
                            stars,
                            truncate(job_title, 38),
                            truncate(keyword, 28)
                        );
                    }
                    println!("\nTotal: {} matches", results.len());
                }
            } else if all {
                // Batch mode: extract keywords from all jobs needing them
                let jobs = db.get_jobs_needing_keywords(force)?;

                if jobs.is_empty() {
                    if force {
                        println!("No jobs with descriptions found.");
                    } else {
                        println!("All jobs with descriptions already have keywords. Use --force to re-extract.");
                    }
                    return Ok(());
                }

                let spec = ai::resolve_model(&model)?;
                let provider = ai::create_provider(&spec)?;

                let total = jobs.len();
                if force {
                    println!("Extracting keywords from {} jobs (--force: re-extracting all, model: {})\n",
                             total, spec.short_name);
                } else {
                    println!("Extracting keywords from {} jobs without keywords (model: {})\n",
                             total, spec.short_name);
                }

                let mut success_count = 0;
                let mut fail_count = 0;

                for (i, job) in jobs.iter().enumerate() {
                    let job_num = i + 1;
                    let employer = job.employer_name.as_deref().unwrap_or("?");
                    print!("[{}/{}] #{} {} at {} ... ",
                           job_num, total, job.id,
                           truncate(&job.title, 40), truncate(employer, 25));

                    let job_text = match &job.raw_text {
                        Some(text) => text,
                        None => {
                            println!("SKIP (no text)");
                            continue;
                        }
                    };

                    match ai::extract_domain_keywords(provider.as_ref(), job_text) {
                        Ok(domain_kw) => {
                            db.add_job_keywords(job.id, &domain_kw.tech, "tech", &spec.short_name)?;
                            db.add_job_keywords(job.id, &domain_kw.discipline, "discipline", &spec.short_name)?;
                            db.add_job_keywords(job.id, &domain_kw.cloud, "cloud", &spec.short_name)?;
                            db.add_job_keywords(job.id, &domain_kw.soft_skill, "soft_skill", &spec.short_name)?;
                            if !domain_kw.profile.is_empty() {
                                db.save_keyword_profile(job.id, &spec.short_name, &domain_kw.profile)?;
                            }
                            let kw_count = domain_kw.tech.len() + domain_kw.discipline.len()
                                + domain_kw.cloud.len() + domain_kw.soft_skill.len();
                            println!("{} keywords", kw_count);
                            success_count += 1;
                        }
                        Err(e) => {
                            println!("FAILED: {}", e);
                            fail_count += 1;
                        }
                    }
                }

                println!("\nDone: {} succeeded, {} failed out of {} jobs",
                         success_count, fail_count, total);
            } else if show {
                // Show stored keywords without re-running AI
                let job_id = job_id.unwrap();
                let job = db.get_job(job_id)?
                    .ok_or_else(|| anyhow!("Job #{} not found", job_id))?;

                let source_model = db.get_latest_keyword_model(job_id)?;
                let source_model = match &source_model {
                    Some(m) => m.as_str(),
                    None => {
                        println!("No stored keywords for job #{}. Run 'hunt keywords {}' to extract.", job_id, job_id);
                        return Ok(());
                    }
                };

                let keywords = db.get_job_keywords(job_id, Some(source_model))?;

                println!("Keywords for job #{}: {} (model: {})\n",
                         job_id, job.title, source_model);

                display_domain_keywords(&keywords);

                // Show profile if available
                if let Some(profile) = db.get_keyword_profile(job_id)? {
                    println!("  PROFILE");
                    for line in textwrap::fill(&profile.profile, 72).lines() {
                        println!("  {}", line);
                    }
                    println!();
                }
            } else {
                // Extract mode: call AI and store results
                let job_id = job_id.unwrap();
                let job = db.get_job(job_id)?
                    .ok_or_else(|| anyhow!("Job #{} not found", job_id))?;

                let job_text = job.raw_text
                    .as_ref()
                    .ok_or_else(|| anyhow!("Job #{} has no raw text to extract keywords from", job_id))?;

                let spec = ai::resolve_model(&model)?;
                let provider = ai::create_provider(&spec)?;

                println!("Extracting keywords from job #{}: {} (model: {})...\n",
                         job_id, job.title, spec.short_name);

                let domain_kw = ai::extract_domain_keywords(provider.as_ref(), job_text)?;

                // Store in database
                db.add_job_keywords(job_id, &domain_kw.tech, "tech", &spec.short_name)?;
                db.add_job_keywords(job_id, &domain_kw.discipline, "discipline", &spec.short_name)?;
                db.add_job_keywords(job_id, &domain_kw.cloud, "cloud", &spec.short_name)?;
                db.add_job_keywords(job_id, &domain_kw.soft_skill, "soft_skill", &spec.short_name)?;

                if !domain_kw.profile.is_empty() {
                    db.save_keyword_profile(job_id, &spec.short_name, &domain_kw.profile)?;
                }

                // Display results — show only what we just stored
                let all_keywords = db.get_job_keywords(job_id, Some(&spec.short_name))?;
                println!("Keywords for job #{}: {} (model: {})\n",
                         job_id, job.title, spec.short_name);

                display_domain_keywords(&all_keywords);

                if !domain_kw.profile.is_empty() {
                    println!("  PROFILE");
                    for line in textwrap::fill(&domain_kw.profile, 72).lines() {
                        println!("  {}", line);
                    }
                    println!();
                }

                let total = domain_kw.tech.len() + domain_kw.discipline.len()
                    + domain_kw.cloud.len() + domain_kw.soft_skill.len();
                println!("Total: {} keywords stored (model: {})", total, spec.short_name);
            }
        }

        Commands::Fit { job_id, resume, model } => {
            db.ensure_initialized()?;
            let job = db.get_job(job_id)?
                .ok_or_else(|| anyhow!("Job #{} not found", job_id))?;

            let job_text = job.raw_text
                .as_ref()
                .ok_or_else(|| anyhow!("Job #{} has no raw text for fit analysis", job_id))?;

            let base_resume = if let Ok(id) = resume.parse::<i64>() {
                db.get_base_resume(id)?
            } else {
                db.get_base_resume_by_name(&resume)?
            }
            .ok_or_else(|| anyhow!("Resume '{}' not found", resume))?;

            let spec = ai::resolve_model(&model)?;
            let provider = ai::create_provider(&spec)?;

            println!("Analyzing fit for job #{}: {} (model: {})...\n", job_id, job.title, spec.short_name);

            let fit = ai::analyze_fit(provider.as_ref(), &base_resume.content, job_text, &job.title)?;

            // Store in database
            db.save_fit_analysis(
                job_id,
                base_resume.id,
                &spec.short_name,
                fit.fit_score,
                &fit.strong_matches,
                &fit.gaps,
                &fit.stretch_areas,
                &fit.narrative,
            )?;

            println!("=== Fit Analysis ===\n");
            println!("Fit Score: {:.0}/100\n", fit.fit_score);

            if !fit.strong_matches.is_empty() {
                println!("Strong Matches:");
                for item in &fit.strong_matches {
                    println!("  + {}", item);
                }
                println!();
            }

            if !fit.gaps.is_empty() {
                println!("Gaps:");
                for item in &fit.gaps {
                    println!("  - {}", item);
                }
                println!();
            }

            if !fit.stretch_areas.is_empty() {
                println!("Stretch Areas:");
                for item in &fit.stretch_areas {
                    println!("  ~ {}", item);
                }
                println!();
            }

            if !fit.narrative.is_empty() {
                println!("Narrative:\n{}", fit.narrative);
            }

            println!("\n(Stored in DB, model: {})", spec.short_name);
        }

        Commands::Browse { status, employer } => {
            db.ensure_initialized()?;
            tui::run_browse(&db, status.as_deref(), employer.as_deref())?;
        }

        Commands::Refresh { username, password_file, days, model, headless, delay } => {
            db.ensure_initialized()?;

            // Step 1: Email ingestion
            println!("═══ Step 1: Fetching job alerts from email ═══\n");
            let password_path = if password_file.starts_with("~/") {
                let home = std::env::var("HOME").unwrap_or_default();
                PathBuf::from(format!("{}/{}", home, &password_file[2..]))
            } else {
                PathBuf::from(&password_file)
            };

            println!("Connecting to Gmail as {}...", username);
            match EmailConfig::from_gmail_password_file(&username, &password_path) {
                Ok(config) => {
                    let ingester = EmailIngester::new(config);
                    println!("Searching for job alerts from the last {} days...", days);
                    match ingester.fetch_job_alerts(&db, days, false) {
                        Ok(stats) => {
                            println!("  Emails processed: {}", stats.emails_found);
                            println!("  Jobs added:       {}", stats.jobs_added);
                            println!("  Duplicates:       {}", stats.duplicates);
                            if stats.errors > 0 {
                                println!("  Errors:           {}", stats.errors);
                            }
                        }
                        Err(e) => println!("  Email fetch failed: {}", e),
                    }
                }
                Err(e) => println!("  Skipping email: {}", e),
            }

            // Step 2: Fetch job descriptions
            println!("\n═══ Step 2: Fetching job descriptions ═══\n");
            let jobs_to_fetch = db.get_jobs_to_fetch(None, false)?;
            if jobs_to_fetch.is_empty() {
                println!("All jobs already have descriptions.");
            } else {
                println!("Fetching descriptions for {} unfetched jobs...\n", jobs_to_fetch.len());
                let mut success = 0;
                let mut fail = 0;

                for (i, job) in jobs_to_fetch.iter().enumerate() {
                    let employer = job.employer_name.as_deref().unwrap_or("?");
                    print!("[{}/{}] #{} {} at {} ... ",
                           i + 1, jobs_to_fetch.len(), job.id,
                           truncate(&job.title, 35), truncate(employer, 20));

                    if let Some(url) = &job.url {
                        match fetch_job_description(url, headless) {
                            Ok(desc) => {
                                let _ = db.update_job_description(job.id, &desc.text, desc.pay_min, desc.pay_max);
                                if let Some(ref emp_name) = desc.employer_name {
                                    let _ = db.update_job_employer(job.id, emp_name);
                                }
                                if desc.no_longer_accepting {
                                    let _ = db.update_job_status(job.id, "closed");
                                }
                                println!("{} chars", desc.text.len());
                                success += 1;
                            }
                            Err(e) => {
                                println!("FAILED: {}", e);
                                fail += 1;
                            }
                        }
                    } else {
                        println!("no URL");
                        fail += 1;
                    }

                    if i + 1 < jobs_to_fetch.len() {
                        let wait = add_jitter(delay);
                        countdown(wait);
                    }
                }
                println!("\n  Fetched: {}, Failed: {}", success, fail);
            }

            // Step 3: Extract keywords
            println!("\n═══ Step 3: Extracting keywords ═══\n");
            let jobs_needing = db.get_jobs_needing_keywords(false)?;
            if jobs_needing.is_empty() {
                println!("All jobs with descriptions already have keywords.");
            } else {
                let spec = ai::resolve_model(&model)?;
                let provider = ai::create_provider(&spec)?;
                println!("Extracting keywords from {} jobs (model: {})\n",
                         jobs_needing.len(), spec.short_name);

                let mut success = 0;
                let mut fail = 0;

                for (i, job) in jobs_needing.iter().enumerate() {
                    let employer = job.employer_name.as_deref().unwrap_or("?");
                    print!("[{}/{}] #{} {} at {} ... ",
                           i + 1, jobs_needing.len(), job.id,
                           truncate(&job.title, 35), truncate(employer, 20));

                    if let Some(text) = &job.raw_text {
                        match ai::extract_domain_keywords(provider.as_ref(), text) {
                            Ok(kw) => {
                                let _ = db.add_job_keywords(job.id, &kw.tech, "tech", &spec.short_name);
                                let _ = db.add_job_keywords(job.id, &kw.discipline, "discipline", &spec.short_name);
                                let _ = db.add_job_keywords(job.id, &kw.cloud, "cloud", &spec.short_name);
                                let _ = db.add_job_keywords(job.id, &kw.soft_skill, "soft_skill", &spec.short_name);
                                if !kw.profile.is_empty() {
                                    let _ = db.save_keyword_profile(job.id, &spec.short_name, &kw.profile);
                                }
                                let count = kw.tech.len() + kw.discipline.len()
                                    + kw.cloud.len() + kw.soft_skill.len();
                                println!("{} keywords", count);
                                success += 1;
                            }
                            Err(e) => {
                                println!("FAILED: {}", e);
                                fail += 1;
                            }
                        }
                    } else {
                        println!("no text");
                    }
                }
                println!("\n  Extracted: {}, Failed: {}", success, fail);
            }

            println!("\n═══ Refresh complete ═══");
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

fn fetch_job_description(url: &str, headless: bool) -> Result<browser::JobDescription> {
    // Use browser automation to fetch job description
    // This handles JavaScript-rendered content and "Show more" buttons
    println!("Initializing browser...");

    // Create a tokio runtime to run async code
    let rt = tokio::runtime::Runtime::new()
        .context("Failed to create tokio runtime")?;

    rt.block_on(async {
        let fetcher = browser::JobFetcher::new(headless)
            .await
            .context("Failed to initialize browser. Make sure geckodriver is running.\n\
                     Start it with: geckodriver --port 4444")?;

        fetcher.fetch_job_description(url).await
    })
}

fn add_jitter(seconds: u64) -> u64 {
    use rand::Rng;
    let jitter = ((seconds as f64) * 0.2) as u64; // ±20%
    let min = seconds.saturating_sub(jitter);
    let max = seconds + jitter;
    rand::thread_rng().gen_range(min..=max)
}

fn countdown(seconds: u64) {
    use std::io::{self, Write};
    print!("Waiting {} seconds before next fetch... ", seconds);
    io::stdout().flush().unwrap();

    for i in (1..=seconds).rev() {
        print!("{}... ", i);
        io::stdout().flush().unwrap();
        std::thread::sleep(std::time::Duration::from_secs(1));
    }
    println!();
}
