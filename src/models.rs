use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Employer {
    pub id: i64,
    pub name: String,
    pub domain: Option<String>,
    pub status: String, // "ok", "yuck", "never"
    pub notes: Option<String>,
    pub created_at: String,
    pub updated_at: String,
    // Startup research fields
    pub crunchbase_url: Option<String>,
    pub funding_stage: Option<String>,
    pub total_funding: Option<i64>,
    pub last_funding_date: Option<String>,
    pub yc_batch: Option<String>,
    pub yc_url: Option<String>,
    pub hn_mentions_count: Option<i64>,
    pub recent_news: Option<String>,
    pub research_updated_at: Option<String>,
    // Public company research fields
    pub controversies: Option<String>,
    pub labor_practices: Option<String>,
    pub environmental_issues: Option<String>,
    pub political_donations: Option<String>,
    pub evil_summary: Option<String>,
    pub public_research_updated_at: Option<String>,
    // Private company ownership fields
    pub parent_company: Option<String>,
    pub pe_owner: Option<String>,
    pub pe_firm_url: Option<String>,
    pub vc_investors: Option<String>,
    pub key_investors: Option<String>,
    pub ownership_concerns: Option<String>,
    pub ownership_type: Option<String>,
    pub ownership_research_updated: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Job {
    pub id: i64,
    pub employer_id: Option<i64>,
    pub employer_name: Option<String>, // denormalized for convenience
    pub title: String,
    pub url: Option<String>,
    pub source: Option<String>, // "linkedin", "indeed", "manual", etc.
    pub status: String,         // "new", "reviewing", "applied", "rejected", "closed"
    pub pay_min: Option<i64>,
    pub pay_max: Option<i64>,
    pub job_code: Option<String>, // Job code/number/requisition ID for deduplication
    pub raw_text: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JobSnapshot {
    pub id: i64,
    pub job_id: i64,
    pub raw_text: String,
    pub captured_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BaseResume {
    pub id: i64,
    pub name: String,
    pub format: String, // "markdown", "plain", "json", "latex"
    pub content: String,
    pub notes: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResumeVariant {
    pub id: i64,
    pub base_resume_id: i64,
    pub job_id: i64,
    pub content: String,
    pub tailoring_notes: Option<String>,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GlassdoorReview {
    pub id: i64,
    pub employer_id: i64,
    pub employer_name: Option<String>,
    pub rating: f64,
    pub title: Option<String>,
    pub pros: Option<String>,
    pub cons: Option<String>,
    pub review_text: Option<String>,
    pub sentiment: String, // "positive", "negative", "neutral"
    pub review_date: Option<String>,
    pub captured_at: String,
}
