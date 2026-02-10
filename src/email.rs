use anyhow::{anyhow, Context, Result};
use mailparse::{parse_mail, MailHeaderMap};
use scraper::{Html, Selector};
use std::collections::HashSet;
use std::fs;
use std::path::Path;

use crate::db::{Database, extract_pay_range};

pub struct EmailConfig {
    pub server: String,
    pub port: u16,
    pub username: String,
    pub password: String,
}

impl EmailConfig {
    pub fn gmail(username: &str, app_password: &str) -> Self {
        Self {
            server: "imap.gmail.com".to_string(),
            port: 993,
            username: username.to_string(),
            password: app_password.trim().to_string(),
        }
    }

    pub fn from_gmail_password_file(username: &str, password_file: &Path) -> Result<Self> {
        let password = fs::read_to_string(password_file)
            .with_context(|| format!("Failed to read password file: {:?}", password_file))?;
        Ok(Self::gmail(username, &password))
    }
}

pub struct EmailIngester {
    config: EmailConfig,
}

impl EmailIngester {
    pub fn new(config: EmailConfig) -> Self {
        Self { config }
    }

    pub fn fetch_job_alerts(&self, db: &Database, days: u32, dry_run: bool) -> Result<IngestStats> {
        let tls = native_tls::TlsConnector::builder().build()?;

        let addr = (self.config.server.as_str(), self.config.port);
        let tcp = std::net::TcpStream::connect(addr)
            .context("Failed to connect to IMAP server")?;
        tcp.set_read_timeout(Some(std::time::Duration::from_secs(30)))?;
        tcp.set_write_timeout(Some(std::time::Duration::from_secs(30)))?;
        let tls_stream = tls.connect(&self.config.server, tcp)?;

        let client = imap::Client::new(tls_stream);
        let mut session = client
            .login(&self.config.username, &self.config.password)
            .map_err(|e| anyhow!("Login failed: {}", e.0))?;

        session.select("INBOX")?;

        let since_date = chrono::Utc::now() - chrono::Duration::days(days as i64);
        let date_str = since_date.format("%d-%b-%Y").to_string();

        let search_queries = vec![
            ("LinkedIn alerts", format!("FROM \"jobs-noreply@linkedin.com\" SINCE {}", date_str)),
            ("LinkedIn jobs", format!("FROM \"linkedin.com\" SUBJECT \"job\" SINCE {}", date_str)),
            ("Indeed", format!("FROM \"indeed.com\" SINCE {}", date_str)),
        ];

        let mut stats = IngestStats::default();
        let mut seen_message_ids: HashSet<String> = HashSet::new();

        for (label, query) in &search_queries {
            eprint!("  {} ... ", label);
            let message_ids = match session.search(query) {
                Ok(ids) => ids,
                Err(e) => {
                    eprintln!("failed: {}", e);
                    continue;
                }
            };

            let new_ids: Vec<_> = message_ids.iter()
                .filter(|id| seen_message_ids.insert(id.to_string()))
                .collect();
            eprintln!("{} emails", new_ids.len());

            for id in new_ids {
                stats.emails_found += 1;

                let messages = session.fetch(id.to_string(), "RFC822")?;
                for message in messages.iter() {
                    if let Some(body) = message.body() {
                        match self.process_email(body, db, dry_run) {
                            Ok(jobs_added) => {
                                stats.jobs_added += jobs_added;
                            }
                            Err(e) => {
                                stats.errors += 1;
                                eprintln!("  Error processing email: {}", e);
                            }
                        }
                    }
                }
            }
        }

        session.logout()?;
        Ok(stats)
    }

    fn process_email(&self, raw: &[u8], db: &Database, dry_run: bool) -> Result<usize> {
        let parsed = parse_mail(raw)?;

        let from = parsed
            .headers
            .get_first_value("From")
            .unwrap_or_default()
            .to_lowercase();
        let subject = parsed
            .headers
            .get_first_value("Subject")
            .unwrap_or_default();

        // Get email body (prefer HTML)
        let body = get_email_body(&parsed)?;

        // Determine source and parse accordingly
        let jobs = if from.contains("linkedin.com") {
            parse_linkedin_email(&subject, &body)?
        } else if from.contains("indeed.com") {
            parse_indeed_email(&subject, &body)?
        } else {
            // Generic parsing
            parse_generic_job_email(&subject, &body)?
        };

        let mut added = 0;
        for job in jobs {
            if dry_run {
                println!(
                    "[DRY RUN] Would add: {} at {} ({})",
                    job.title,
                    job.employer.as_deref().unwrap_or("Unknown"),
                    job.source
                );
            } else {
                // Check for duplicates by URL or title+employer
                if !job_exists(db, &job)? {
                    add_job_from_email(db, &job)?;
                    added += 1;
                }
            }
        }

        Ok(added)
    }
}

fn get_email_body(parsed: &mailparse::ParsedMail) -> Result<String> {
    // Try to find HTML part first, then plain text
    if parsed.subparts.is_empty() {
        // Single part email
        let body = parsed.get_body()?;
        return Ok(body);
    }

    // Multipart email - look for HTML
    for part in &parsed.subparts {
        let content_type = part
            .headers
            .get_first_value("Content-Type")
            .unwrap_or_default();
        if content_type.contains("text/html") {
            return Ok(part.get_body()?);
        }
    }

    // Fallback to plain text
    for part in &parsed.subparts {
        let content_type = part
            .headers
            .get_first_value("Content-Type")
            .unwrap_or_default();
        if content_type.contains("text/plain") {
            return Ok(part.get_body()?);
        }
    }

    // Last resort - first part
    if let Some(part) = parsed.subparts.first() {
        return Ok(part.get_body()?);
    }

    Err(anyhow!("No email body found"))
}

#[derive(Debug, Clone)]
pub struct ParsedJob {
    pub title: String,
    pub employer: Option<String>,
    pub url: Option<String>,
    #[allow(dead_code)]
    pub location: Option<String>,
    pub pay_min: Option<i64>,
    pub pay_max: Option<i64>,
    pub source: String,
    pub raw_text: String,
}

fn is_navigation_artifact(text: &str) -> bool {
    let text_lower = text.to_lowercase();
    let text_trimmed = text.trim();

    // Filter short titles (< 10 chars)
    if text_trimmed.len() < 10 {
        return true;
    }

    // Filter exact matches (case-insensitive)
    let artifacts = [
        "search for jobs",
        "see all jobs",
        "view all",
        "search other jobs",
        "jobs",
    ];

    for artifact in &artifacts {
        if text_lower == *artifact {
            return true;
        }
    }

    // Filter patterns
    if text_lower.starts_with("jobs similar to")
        || text_lower.starts_with("jobs in ")
        || text_lower.starts_with("manage job")
        || text_lower.contains("unsubscribe")
        || text_lower.contains("privacy")
    {
        return true;
    }

    // Filter titles ending in " jobs" (e.g., "Engineering Manager jobs")
    // These are usually links to search results, not actual job postings
    if text_trimmed.ends_with(" jobs") || text_trimmed.ends_with(" Jobs") {
        return true;
    }

    false
}

pub fn is_search_link(url: &str) -> bool {
    // Filter non-job LinkedIn/Indeed URLs (search, alerts, settings, etc.)
    // Examples:
    // - https://www.linkedin.com/comm/jobs/search
    // - https://www.linkedin.com/comm/jobs/search?keywords=...
    // - https://www.linkedin.com/comm/jobs/alerts
    url.contains("/jobs/search") || url.contains("/search?") || url.contains("/jobs/alerts")
}

fn parse_linkedin_email(_subject: &str, body: &str) -> Result<Vec<ParsedJob>> {
    let mut jobs = Vec::new();
    let document = Html::parse_document(body);

    // LinkedIn job alert emails have job cards with specific structure
    // Try multiple selectors as LinkedIn changes their email format

    // Selector for job titles (usually in <a> tags with job URLs)
    let job_link_selector = Selector::parse("a[href*='linkedin.com/comm/jobs']").ok();
    let _job_card_selector = Selector::parse("table[role='presentation']").ok();

    // Try to extract from links first
    if let Some(ref selector) = job_link_selector {
        for element in document.select(selector) {
            let href = element.value().attr("href").unwrap_or("");
            let text = element.text().collect::<Vec<_>>().join(" ");
            let text = text.trim();

            if text.is_empty() {
                continue;
            }

            // Skip navigation artifacts
            if is_navigation_artifact(text) {
                continue;
            }

            // Skip search result links
            if is_search_link(href) {
                continue;
            }

            // Try to parse LinkedIn format with location first, then fallback to other patterns
            let (title, employer, location) = if let Some((t, e, l)) = parse_linkedin_title_company_location(text) {
                (t, e, l)
            } else {
                let (t, e) = parse_title_at_company(text);
                (t, e, None)
            };

            if !title.is_empty() {
                let (pay_min, pay_max) = extract_pay_range(text);
                jobs.push(ParsedJob {
                    title,
                    employer,
                    url: clean_tracking_url(href),
                    location,
                    pay_min,
                    pay_max,
                    source: "linkedin".to_string(),
                    raw_text: text.to_string(),
                });
            }
        }
    }

    // If no jobs found, try generic text extraction
    if jobs.is_empty() {
        // Extract text and look for patterns
        let text = document.root_element().text().collect::<Vec<_>>().join(" ");
        jobs.extend(extract_jobs_from_text(&text, "linkedin")?);
    }

    // Deduplicate by title
    jobs.dedup_by(|a, b| a.title.to_lowercase() == b.title.to_lowercase());

    Ok(jobs)
}

fn parse_indeed_email(_subject: &str, body: &str) -> Result<Vec<ParsedJob>> {
    let mut jobs = Vec::new();
    let document = Html::parse_document(body);

    // Indeed emails typically have job links
    let job_link_selector = Selector::parse("a[href*='indeed.com']").ok();

    if let Some(ref selector) = job_link_selector {
        for element in document.select(selector) {
            let href = element.value().attr("href").unwrap_or("");
            let text = element.text().collect::<Vec<_>>().join(" ");
            let text = text.trim();

            if text.is_empty() {
                continue;
            }

            // Skip navigation artifacts
            if is_navigation_artifact(text) {
                continue;
            }

            // Skip search result links
            if is_search_link(href) {
                continue;
            }

            // Check if this looks like a job link
            if href.contains("/viewjob") || href.contains("/rc/clk") || href.contains("jk=") {
                let (title, employer) = parse_title_at_company(text);

                if !title.is_empty() {
                    let (pay_min, pay_max) = extract_pay_range(text);
                    jobs.push(ParsedJob {
                        title,
                        employer,
                        url: clean_tracking_url(href),
                        location: None,
                        pay_min,
                        pay_max,
                        source: "indeed".to_string(),
                        raw_text: text.to_string(),
                    });
                }
            }
        }
    }

    // Deduplicate
    jobs.dedup_by(|a, b| a.title.to_lowercase() == b.title.to_lowercase());

    Ok(jobs)
}

fn parse_generic_job_email(_subject: &str, body: &str) -> Result<Vec<ParsedJob>> {
    let document = Html::parse_document(body);
    let text = document.root_element().text().collect::<Vec<_>>().join(" ");
    extract_jobs_from_text(&text, "email")
}

fn extract_jobs_from_text(text: &str, source: &str) -> Result<Vec<ParsedJob>> {
    let mut jobs = Vec::new();

    // Look for common job title patterns
    let title_patterns = [
        r"(?i)(senior|staff|principal|lead|junior|sr\.?|jr\.?)?\s*(software|devops|platform|infrastructure|site reliability|sre|cloud|backend|frontend|full[- ]?stack|data|ml|machine learning)\s*(engineer|developer|architect|manager|lead|specialist)",
    ];

    let re = regex::Regex::new(title_patterns[0])?;

    for cap in re.captures_iter(text) {
        let title = cap.get(0).map(|m| m.as_str().trim().to_string());
        if let Some(t) = title {
            if t.len() > 5 {
                let (pay_min, pay_max) = extract_pay_range(text);
                jobs.push(ParsedJob {
                    title: t,
                    employer: None,
                    url: None,
                    location: None,
                    pay_min,
                    pay_max,
                    source: source.to_string(),
                    raw_text: text.chars().take(500).collect(),
                });
            }
        }
    }

    jobs.dedup_by(|a, b| a.title.to_lowercase() == b.title.to_lowercase());
    Ok(jobs)
}

fn parse_linkedin_title_company_location(text: &str) -> Option<(String, Option<String>, Option<String>)> {
    // LinkedIn format: "Title             Company · Location"
    // Multiple spaces separate title from company
    // Middot (·) separates company from location

    let text = text.trim();

    // Look for middot (·) to separate company and location
    if let Some(middot_idx) = text.find('·') {
        let before_middot = &text[..middot_idx].trim();
        let location = text[middot_idx + '·'.len_utf8()..].trim().to_string();

        // Split before middot by multiple spaces (2+)
        // Use regex to find the last occurrence of 2+ spaces
        let re = regex::Regex::new(r"\s{2,}").ok()?;
        let mut last_match = None;
        for mat in re.find_iter(before_middot) {
            last_match = Some(mat);
        }

        if let Some(space_match) = last_match {
            let title = before_middot[..space_match.start()].trim().to_string();
            let company = before_middot[space_match.end()..].trim().to_string();

            if !title.is_empty() && !company.is_empty() {
                return Some((title, Some(company), Some(location)));
            }
        }
    }

    None
}

fn parse_title_at_company(text: &str) -> (String, Option<String>) {
    // Common patterns:
    // "Software Engineer at Google"
    // "Software Engineer, Google"
    // "Software Engineer - Google"
    // "Title             Company · Location" (LinkedIn specific)

    let text = text.trim();

    // Try LinkedIn format first (most specific)
    if let Some((title, company, _location)) = parse_linkedin_title_company_location(text) {
        return (title, company);
    }

    // Try " at " pattern
    if let Some(idx) = text.to_lowercase().find(" at ") {
        let title = text[..idx].trim().to_string();
        let employer = text[idx + 4..].trim().to_string();
        if !employer.is_empty() {
            return (title, Some(employer));
        }
    }

    // Try " - " pattern (but be careful with job title hyphens)
    if let Some(idx) = text.rfind(" - ") {
        let title = text[..idx].trim().to_string();
        let employer = text[idx + 3..].trim().to_string();
        // Only use if employer part doesn't look like part of title
        if !employer.is_empty()
            && !employer.to_lowercase().contains("engineer")
            && !employer.to_lowercase().contains("developer")
        {
            return (title, Some(employer));
        }
    }

    // Try ", " pattern (last comma)
    if let Some(idx) = text.rfind(", ") {
        let potential_employer = text[idx + 2..].trim();
        // Check if it looks like a company (not a location indicator)
        if !potential_employer.is_empty()
            && potential_employer.len() < 50
            && !potential_employer.contains("Remote")
            && !potential_employer.contains("Hybrid")
        {
            let title = text[..idx].trim().to_string();
            return (title, Some(potential_employer.to_string()));
        }
    }

    // No pattern matched - just return title
    (text.to_string(), None)
}

fn clean_tracking_url(url: &str) -> Option<String> {
    // LinkedIn and Indeed wrap URLs in tracking redirects
    // Strip query parameters (everything after ?) as they are tracking garbage
    if url.is_empty() {
        return None;
    }

    // Remove everything after the ? (query parameters)
    let clean_url = if let Some(idx) = url.find('?') {
        &url[..idx]
    } else {
        url
    };

    Some(clean_url.to_string())
}

fn job_exists(db: &Database, job: &ParsedJob) -> Result<bool> {
    // Use sophisticated duplicate detection
    let duplicate_id = db.is_duplicate_job(
        &job.title,
        job.employer.as_deref(),
        job.url.as_deref(),
    )?;

    Ok(duplicate_id.is_some())
}

fn add_job_from_email(db: &Database, job: &ParsedJob) -> Result<i64> {
    db.add_job_full(
        &job.title,
        job.employer.as_deref(),
        job.url.as_deref(),
        Some(&job.source),
        job.pay_min,
        job.pay_max,
        Some(&job.raw_text),
    )
}

#[derive(Debug, Default)]
pub struct IngestStats {
    pub emails_found: usize,
    pub jobs_added: usize,
    pub errors: usize,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_linkedin_title_company_location() {
        // Test case 1: Staff DevOps Engineer, DevInfra             SandboxAQ · United States (Remote)
        let input1 = "Staff DevOps Engineer, DevInfra             SandboxAQ · United States (Remote)";
        let result1 = parse_linkedin_title_company_location(input1);
        assert!(result1.is_some());
        let (title1, company1, location1) = result1.unwrap();
        assert_eq!(title1, "Staff DevOps Engineer, DevInfra");
        assert_eq!(company1, Some("SandboxAQ".to_string()));
        assert_eq!(location1, Some("United States (Remote)".to_string()));

        // Test case 2: Senior Platform Engineer             Sully.ai · Mountain View, CA (Remote)
        let input2 = "Senior Platform Engineer             Sully.ai · Mountain View, CA (Remote)";
        let result2 = parse_linkedin_title_company_location(input2);
        assert!(result2.is_some());
        let (title2, company2, location2) = result2.unwrap();
        assert_eq!(title2, "Senior Platform Engineer");
        assert_eq!(company2, Some("Sully.ai".to_string()));
        assert_eq!(location2, Some("Mountain View, CA (Remote)".to_string()));

        // Test case 3: Staff Engineer - Platform             Grow Therapy · New York, NY (Remote)
        let input3 = "Staff Engineer - Platform             Grow Therapy · New York, NY (Remote)";
        let result3 = parse_linkedin_title_company_location(input3);
        assert!(result3.is_some());
        let (title3, company3, location3) = result3.unwrap();
        assert_eq!(title3, "Staff Engineer - Platform");
        assert_eq!(company3, Some("Grow Therapy".to_string()));
        assert_eq!(location3, Some("New York, NY (Remote)".to_string()));
    }

    #[test]
    fn test_parse_linkedin_title_company_location_no_middot() {
        // Should return None if no middot present
        let input = "Senior Engineer at Google";
        let result = parse_linkedin_title_company_location(input);
        assert!(result.is_none());
    }

    #[test]
    fn test_parse_linkedin_title_company_location_no_multiple_spaces() {
        // Should return None if no multiple spaces before middot
        let input = "Senior Engineer Company · Location";
        let result = parse_linkedin_title_company_location(input);
        assert!(result.is_none());
    }

    #[test]
    fn test_parse_title_at_company_with_linkedin_format() {
        // Should parse LinkedIn format through parse_title_at_company
        let input = "Staff DevOps Engineer, DevInfra             SandboxAQ · United States (Remote)";
        let (title, company) = parse_title_at_company(input);
        assert_eq!(title, "Staff DevOps Engineer, DevInfra");
        assert_eq!(company, Some("SandboxAQ".to_string()));
    }

    #[test]
    fn test_parse_title_at_company_fallback_patterns() {
        // Test " at " pattern
        let (title1, company1) = parse_title_at_company("Software Engineer at Google");
        assert_eq!(title1, "Software Engineer");
        assert_eq!(company1, Some("Google".to_string()));

        // Test " - " pattern
        let (title2, company2) = parse_title_at_company("DevOps Lead - Amazon");
        assert_eq!(title2, "DevOps Lead");
        assert_eq!(company2, Some("Amazon".to_string()));
    }

    #[test]
    fn test_is_navigation_artifact_filters_short_titles() {
        assert!(is_navigation_artifact("Jobs"));
        assert!(is_navigation_artifact("View"));
        assert!(is_navigation_artifact("Search"));
        assert!(is_navigation_artifact("abc"));
    }

    #[test]
    fn test_is_navigation_artifact_filters_exact_matches() {
        assert!(is_navigation_artifact("Search for jobs"));
        assert!(is_navigation_artifact("SEARCH FOR JOBS"));
        assert!(is_navigation_artifact("See all jobs"));
        assert!(is_navigation_artifact("View all"));
        assert!(is_navigation_artifact("Search other jobs"));
        assert!(is_navigation_artifact("Jobs"));
    }

    #[test]
    fn test_is_navigation_artifact_filters_patterns() {
        assert!(is_navigation_artifact("Jobs similar to Senior Engineer"));
        assert!(is_navigation_artifact("Jobs in Bellevue"));
        assert!(is_navigation_artifact("Jobs in Seattle, WA"));
        assert!(is_navigation_artifact("Unsubscribe from alerts"));
        assert!(is_navigation_artifact("Privacy settings"));
        assert!(is_navigation_artifact("Manage job alerts"));
    }

    #[test]
    fn test_is_navigation_artifact_allows_valid_jobs() {
        assert!(!is_navigation_artifact("Staff DevOps Engineer, DevInfra SandboxAQ"));
        assert!(!is_navigation_artifact("Senior Software Engineer at Google"));
        assert!(!is_navigation_artifact("Principal Engineer - Cloud Infrastructure"));
        assert!(!is_navigation_artifact("Site Reliability Engineer"));
        assert!(!is_navigation_artifact("Full Stack Developer at Microsoft"));
    }

    #[test]
    fn test_is_navigation_artifact_edge_cases() {
        // Exactly 10 chars should not be filtered
        assert!(!is_navigation_artifact("1234567890"));
        // 9 chars should be filtered
        assert!(is_navigation_artifact("123456789"));
        // Empty string should be filtered
        assert!(is_navigation_artifact(""));
        // Whitespace only should be filtered (trimmed to empty)
        assert!(is_navigation_artifact("   "));
    }

    #[test]
    fn test_is_navigation_artifact_filters_search_titles() {
        // Filter titles ending in " jobs" - these are search result links
        assert!(is_navigation_artifact("Engineering Manager jobs"));
        assert!(is_navigation_artifact("Full Stack Engineer jobs"));
        assert!(is_navigation_artifact("Software Developer jobs"));
        assert!(is_navigation_artifact("DevOps Engineer Jobs"));

        // But allow actual job titles with "jobs" in the middle
        assert!(!is_navigation_artifact("Jobs Program Manager at Google"));
        assert!(!is_navigation_artifact("Steve Jobs Memorial Engineer"));
    }

    #[test]
    fn test_is_search_link() {
        // LinkedIn search URLs
        assert!(is_search_link("https://www.linkedin.com/comm/jobs/search"));
        assert!(is_search_link("https://www.linkedin.com/comm/jobs/search?keywords=Engineering+Manager"));
        assert!(is_search_link("https://www.linkedin.com/jobs/search?keywords=test"));

        // LinkedIn alerts URLs
        assert!(is_search_link("https://www.linkedin.com/comm/jobs/alerts"));

        // Indeed search URLs
        assert!(is_search_link("https://www.indeed.com/jobs/search?q=engineer"));

        // Valid job URLs (not search)
        assert!(!is_search_link("https://www.linkedin.com/jobs/view/123456"));
        assert!(!is_search_link("https://www.linkedin.com/comm/jobs/view/123456"));
        assert!(!is_search_link("https://www.indeed.com/viewjob?jk=abc123"));
    }

    #[test]
    fn test_clean_tracking_url_strips_query_params() {
        // Test with query parameters
        let url1 = "https://www.linkedin.com/jobs/view/123456?refId=abcd&trackingId=xyz";
        assert_eq!(
            clean_tracking_url(url1),
            Some("https://www.linkedin.com/jobs/view/123456".to_string())
        );

        // Test with Indeed URL
        let url2 = "https://www.indeed.com/viewjob?jk=123&tk=456&from=email";
        assert_eq!(
            clean_tracking_url(url2),
            Some("https://www.indeed.com/viewjob".to_string())
        );

        // Test URL without query params (should remain unchanged)
        let url3 = "https://jobs.example.com/posting/12345";
        assert_eq!(
            clean_tracking_url(url3),
            Some("https://jobs.example.com/posting/12345".to_string())
        );

        // Test empty URL
        assert_eq!(clean_tracking_url(""), None);

        // Test URL with fragment after query (should strip both)
        let url4 = "https://example.com/job?id=123#section";
        assert_eq!(
            clean_tracking_url(url4),
            Some("https://example.com/job".to_string())
        );
    }
}
