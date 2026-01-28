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
