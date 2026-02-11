use anyhow::{anyhow, Context, Result};
use serde::{Deserialize, Serialize};
use std::env;

// --- Provider trait ---

pub trait AIProvider {
    fn complete(&self, prompt: &str, max_tokens: u32) -> Result<String>;
    #[allow(dead_code)]
    fn model_name(&self) -> &str;
}

#[derive(Debug, Clone)]
pub enum ProviderKind {
    Anthropic,
    OpenAI,
    ClaudeCode,
}

#[derive(Debug, Clone)]
pub struct ModelSpec {
    pub provider: ProviderKind,
    pub model_id: String,
    pub short_name: String,
}

pub fn resolve_model(name: &str) -> Result<ModelSpec> {
    match name {
        // Claude Code provider (uses `claude` CLI — no API key needed)
        "claude-sonnet" | "sonnet" => Ok(ModelSpec {
            provider: ProviderKind::ClaudeCode,
            model_id: "claude-sonnet-4-5-20250929".to_string(),
            short_name: "claude-sonnet".to_string(),
        }),
        "claude-opus" | "opus" => Ok(ModelSpec {
            provider: ProviderKind::ClaudeCode,
            model_id: "claude-opus-4-6".to_string(),
            short_name: "claude-opus".to_string(),
        }),
        "claude-haiku" | "haiku" => Ok(ModelSpec {
            provider: ProviderKind::ClaudeCode,
            model_id: "claude-haiku-4-5-20251001".to_string(),
            short_name: "claude-haiku".to_string(),
        }),
        // Direct Anthropic API (requires ANTHROPIC_API_KEY)
        "api-sonnet" => Ok(ModelSpec {
            provider: ProviderKind::Anthropic,
            model_id: "claude-sonnet-4-5-20250929".to_string(),
            short_name: "api-sonnet".to_string(),
        }),
        "api-opus" => Ok(ModelSpec {
            provider: ProviderKind::Anthropic,
            model_id: "claude-opus-4-6".to_string(),
            short_name: "api-opus".to_string(),
        }),
        "api-haiku" => Ok(ModelSpec {
            provider: ProviderKind::Anthropic,
            model_id: "claude-haiku-4-5-20251001".to_string(),
            short_name: "api-haiku".to_string(),
        }),
        // OpenAI (requires OPENAI_API_KEY)
        "gpt-5.2" | "gpt5" => Ok(ModelSpec {
            provider: ProviderKind::OpenAI,
            model_id: "gpt-5.2".to_string(),
            short_name: "gpt-5.2".to_string(),
        }),
        "gpt-5.2-pro" | "gpt5-pro" => Ok(ModelSpec {
            provider: ProviderKind::OpenAI,
            model_id: "gpt-5.2-pro".to_string(),
            short_name: "gpt-5.2-pro".to_string(),
        }),
        "gpt-4o" => Ok(ModelSpec {
            provider: ProviderKind::OpenAI,
            model_id: "gpt-4o".to_string(),
            short_name: "gpt-4o".to_string(),
        }),
        "o3" => Ok(ModelSpec {
            provider: ProviderKind::OpenAI,
            model_id: "o3".to_string(),
            short_name: "o3".to_string(),
        }),
        _ => Err(anyhow!(
            "Unknown model '{}'. Available: claude-sonnet (default), claude-opus, claude-haiku, \
             api-sonnet, api-opus, api-haiku, gpt-5.2, gpt-5.2-pro, gpt-4o, o3",
            name
        )),
    }
}

pub fn create_provider(spec: &ModelSpec) -> Result<Box<dyn AIProvider>> {
    match spec.provider {
        ProviderKind::ClaudeCode => {
            // Pass short alias (e.g. "sonnet") to claude CLI — full model IDs route through API billing
            let cli_model = match spec.short_name.as_str() {
                "claude-sonnet" => "sonnet",
                "claude-opus" => "opus",
                "claude-haiku" => "haiku",
                _ => &spec.short_name,
            };
            let provider = ClaudeCodeProvider::new(cli_model.to_string())?;
            Ok(Box::new(provider))
        }
        ProviderKind::Anthropic => {
            let provider = AnthropicProvider::new(spec.model_id.clone())?;
            Ok(Box::new(provider))
        }
        ProviderKind::OpenAI => {
            let provider = OpenAIProvider::new(spec.model_id.clone())?;
            Ok(Box::new(provider))
        }
    }
}

// --- Anthropic provider ---

const ANTHROPIC_API_URL: &str = "https://api.anthropic.com/v1/messages";

#[derive(Debug, Serialize)]
struct AnthropicMessage {
    role: String,
    content: String,
}

#[derive(Debug, Serialize)]
struct AnthropicRequest {
    model: String,
    max_tokens: u32,
    messages: Vec<AnthropicMessage>,
}

#[derive(Debug, Deserialize)]
struct AnthropicContentBlock {
    #[allow(dead_code)]
    #[serde(rename = "type")]
    content_type: String,
    text: String,
}

#[derive(Debug, Deserialize)]
struct AnthropicResponse {
    content: Vec<AnthropicContentBlock>,
}

#[derive(Debug)]
pub struct AnthropicProvider {
    api_key: String,
    model_id: String,
    client: reqwest::blocking::Client,
}

impl AnthropicProvider {
    pub fn new(model_id: String) -> Result<Self> {
        let api_key = env::var("ANTHROPIC_API_KEY")
            .context("ANTHROPIC_API_KEY environment variable not set. Set it with: export ANTHROPIC_API_KEY=your-key-here")?;
        let client = reqwest::blocking::Client::builder()
            .timeout(std::time::Duration::from_secs(120))
            .build()?;
        Ok(Self { api_key, model_id, client })
    }
}

impl AIProvider for AnthropicProvider {
    fn complete(&self, prompt: &str, max_tokens: u32) -> Result<String> {
        let request = AnthropicRequest {
            model: self.model_id.clone(),
            max_tokens,
            messages: vec![AnthropicMessage {
                role: "user".to_string(),
                content: prompt.to_string(),
            }],
        };

        let response = self
            .client
            .post(ANTHROPIC_API_URL)
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", "2023-06-01")
            .header("content-type", "application/json")
            .json(&request)
            .send()
            .context("Failed to send request to Anthropic API")?;

        if !response.status().is_success() {
            let status = response.status();
            let error_text = response.text().unwrap_or_default();
            return Err(anyhow!(
                "Anthropic API request failed with status {}: {}",
                status,
                error_text
            ));
        }

        let api_response: AnthropicResponse = response
            .json()
            .context("Failed to parse Anthropic API response")?;

        api_response
            .content
            .first()
            .map(|block| block.text.clone())
            .ok_or_else(|| anyhow!("No content in Anthropic API response"))
    }

    fn model_name(&self) -> &str {
        &self.model_id
    }
}

// --- Claude Code provider (shells out to `claude` CLI) ---

#[derive(Debug)]
pub struct ClaudeCodeProvider {
    model_id: String,
}

impl ClaudeCodeProvider {
    pub fn new(model_id: String) -> Result<Self> {
        // Verify claude CLI is available
        std::process::Command::new("claude")
            .arg("--version")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .context("'claude' CLI not found. Install Claude Code or use api-sonnet/gpt-5.2 instead.")?;
        Ok(Self { model_id })
    }
}

impl AIProvider for ClaudeCodeProvider {
    fn complete(&self, prompt: &str, _max_tokens: u32) -> Result<String> {
        use std::io::Write;
        use std::process::{Command, Stdio};

        let mut child = Command::new("claude")
            .arg("-p")
            .arg("-")
            .arg("--model")
            .arg(&self.model_id)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .context("Failed to start 'claude' CLI")?;

        // Write prompt to stdin
        if let Some(mut stdin) = child.stdin.take() {
            stdin.write_all(prompt.as_bytes())
                .context("Failed to write prompt to claude CLI stdin")?;
        }

        let output = child.wait_with_output()
            .context("Failed to wait for claude CLI")?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            let stdout = String::from_utf8_lossy(&output.stdout);
            let detail = if !stderr.is_empty() {
                stderr.to_string()
            } else if !stdout.is_empty() {
                stdout.to_string()
            } else {
                format!("exit code: {}", output.status)
            };
            return Err(anyhow!("claude CLI failed: {}", detail));
        }

        let response = String::from_utf8(output.stdout)
            .context("Invalid UTF-8 in claude CLI output")?;

        if response.trim().is_empty() {
            return Err(anyhow!("Empty response from claude CLI"));
        }

        Ok(response)
    }

    fn model_name(&self) -> &str {
        &self.model_id
    }
}

// --- OpenAI provider ---

const OPENAI_API_URL: &str = "https://api.openai.com/v1/chat/completions";

#[derive(Debug, Serialize)]
struct OpenAIMessage {
    role: String,
    content: String,
}

#[derive(Debug, Serialize)]
struct OpenAIRequest {
    model: String,
    max_completion_tokens: u32,
    messages: Vec<OpenAIMessage>,
}

#[derive(Debug, Deserialize)]
struct OpenAIResponseMessage {
    content: String,
}

#[derive(Debug, Deserialize)]
struct OpenAIChoice {
    message: OpenAIResponseMessage,
}

#[derive(Debug, Deserialize)]
struct OpenAIResponse {
    choices: Vec<OpenAIChoice>,
}

#[derive(Debug)]
pub struct OpenAIProvider {
    api_key: String,
    model_id: String,
    client: reqwest::blocking::Client,
}

impl OpenAIProvider {
    pub fn new(model_id: String) -> Result<Self> {
        let api_key = env::var("OPENAI_API_KEY")
            .context("OPENAI_API_KEY environment variable not set. Set it with: export OPENAI_API_KEY=your-key-here")?;
        let client = reqwest::blocking::Client::builder()
            .timeout(std::time::Duration::from_secs(120))
            .build()?;
        Ok(Self { api_key, model_id, client })
    }
}

impl AIProvider for OpenAIProvider {
    fn complete(&self, prompt: &str, max_tokens: u32) -> Result<String> {
        let request = OpenAIRequest {
            model: self.model_id.clone(),
            max_completion_tokens: max_tokens,
            messages: vec![OpenAIMessage {
                role: "user".to_string(),
                content: prompt.to_string(),
            }],
        };

        let response = self
            .client
            .post(OPENAI_API_URL)
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("Content-Type", "application/json")
            .json(&request)
            .send()
            .context("Failed to send request to OpenAI API")?;

        if !response.status().is_success() {
            let status = response.status();
            let error_text = response.text().unwrap_or_default();
            return Err(anyhow!(
                "OpenAI API request failed with status {}: {}",
                status,
                error_text
            ));
        }

        let api_response: OpenAIResponse = response
            .json()
            .context("Failed to parse OpenAI API response")?;

        api_response
            .choices
            .first()
            .map(|choice| choice.message.content.clone())
            .ok_or_else(|| anyhow!("No choices in OpenAI API response"))
    }

    fn model_name(&self) -> &str {
        &self.model_id
    }
}

// --- Standalone AI functions ---

pub fn analyze_job(provider: &dyn AIProvider, job_text: &str) -> Result<String> {
    let prompt = format!(
        "Analyze this job posting and provide:\n\
        1. Required skills and experience\n\
        2. Nice-to-have qualifications\n\
        3. Red flags or concerns\n\
        4. Estimated seniority level\n\
        5. Overall assessment (1-10 scale with brief reasoning)\n\n\
        Job posting:\n{}",
        job_text
    );
    provider.complete(&prompt, 4096)
}

#[allow(dead_code)]
pub fn extract_keywords(provider: &dyn AIProvider, job_text: &str) -> Result<Vec<String>> {
    let prompt = format!(
        "Analyze this job posting and extract key technical skills, technologies, and requirements. Return ONLY a comma-separated list of keywords, no explanations.\n\nJob posting:\n{}",
        job_text
    );

    let response = provider.complete(&prompt, 4096)?;

    let keywords: Vec<String> = response
        .split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();

    Ok(keywords)
}

pub struct DomainKeywords {
    pub tech: Vec<(String, i32)>,
    pub discipline: Vec<(String, i32)>,
    pub cloud: Vec<(String, i32)>,
    pub soft_skill: Vec<(String, i32)>,
    pub profile: String,
}

pub fn extract_domain_keywords(
    provider: &dyn AIProvider,
    job_text: &str,
) -> Result<DomainKeywords> {
    let prompt = format!(
        "Extract keywords from this job posting into exactly four domain lines plus a profile.\n\n\
        RULES:\n\
        - Each keyword is 1-3 words MAX (e.g. \"Kubernetes\" not \"Kubernetes container orchestration\")\n\
        - NO duplicates across or within domains\n\
        - Each keyword appears in exactly ONE domain\n\
        - NO descriptions, years of experience, or degree requirements — just the skill/tool name\n\
        - Weight: 3=explicitly required, 2=emphasized, 1=nice-to-have\n\n\
        DOMAINS:\n\
        - TECH: languages, frameworks, databases, tools (Python, Terraform, PostgreSQL, dbt)\n\
        - DISCIPLINE: practices, methodologies, role focus (DevOps, SRE, CI/CD, Agile, microservices)\n\
        - CLOUD: cloud providers and services only (AWS, GCP, Azure, S3, Lambda, EKS)\n\
        - SOFT_SKILL: people skills (leadership, communication, mentoring)\n\n\
        FORMAT — return exactly these 5 lines, nothing else:\n\
        TECH: Kubernetes/3, Python/2, dbt/1\n\
        DISCIPLINE: DevOps/3, SRE/2, Agile/1\n\
        CLOUD: AWS/3, Azure/1\n\
        SOFT_SKILL: leadership/3, communication/2\n\
        PROFILE: 2-3 sentences summarizing what this role emphasizes.\n\n\
        Job posting:\n{}",
        job_text
    );

    let response = provider.complete(&prompt, 4096)?;

    let mut tech = Vec::new();
    let mut discipline = Vec::new();
    let mut cloud = Vec::new();
    let mut soft_skill = Vec::new();
    let mut profile = String::new();

    for line in response.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix("TECH:") {
            tech = parse_weighted_keywords(rest);
        } else if let Some(rest) = line.strip_prefix("DISCIPLINE:") {
            discipline = parse_weighted_keywords(rest);
        } else if let Some(rest) = line.strip_prefix("CLOUD:") {
            cloud = parse_weighted_keywords(rest);
        } else if let Some(rest) = line.strip_prefix("SOFT_SKILL:") {
            soft_skill = parse_weighted_keywords(rest);
        } else if let Some(rest) = line.strip_prefix("PROFILE:") {
            profile = rest.trim().to_string();
        }
    }

    // Deduplicate within each domain (case-insensitive, keep highest weight)
    tech = dedup_keywords(tech);
    discipline = dedup_keywords(discipline);
    cloud = dedup_keywords(cloud);
    soft_skill = dedup_keywords(soft_skill);

    // Deduplicate across domains (keep in first domain seen)
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    for list in [&mut tech, &mut discipline, &mut cloud, &mut soft_skill] {
        list.retain(|(kw, _)| seen.insert(kw.to_lowercase()));
    }

    Ok(DomainKeywords {
        tech,
        discipline,
        cloud,
        soft_skill,
        profile,
    })
}

fn dedup_keywords(keywords: Vec<(String, i32)>) -> Vec<(String, i32)> {
    let mut seen: std::collections::HashMap<String, (String, i32)> = std::collections::HashMap::new();
    for (kw, weight) in keywords {
        let key = kw.to_lowercase();
        let entry = seen.entry(key).or_insert_with(|| (kw.clone(), weight));
        if weight > entry.1 {
            entry.1 = weight;
        }
    }
    let mut result: Vec<(String, i32)> = seen.into_values().collect();
    result.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.to_lowercase().cmp(&b.0.to_lowercase())));
    result
}

fn parse_weighted_keywords(input: &str) -> Vec<(String, i32)> {
    input
        .split(',')
        .filter_map(|s| {
            let s = s.trim();
            if s.is_empty() {
                return None;
            }
            if let Some(slash_pos) = s.rfind('/') {
                let keyword = s[..slash_pos].trim().to_string();
                let weight = s[slash_pos + 1..].trim().parse::<i32>().unwrap_or(2);
                let weight = weight.clamp(1, 3);
                if keyword.is_empty() {
                    None
                } else {
                    Some((keyword, weight))
                }
            } else {
                // No weight specified, default to 2
                Some((s.to_string(), 2))
            }
        })
        .collect()
}

pub struct FitResult {
    pub fit_score: f64,
    pub strong_matches: Vec<String>,
    pub gaps: Vec<String>,
    pub stretch_areas: Vec<String>,
    pub narrative: String,
}

pub fn analyze_fit(
    provider: &dyn AIProvider,
    resume: &str,
    job_text: &str,
    title: &str,
) -> Result<FitResult> {
    let prompt = format!(
        "Compare this resume against the job posting and provide a fit analysis.\n\n\
        Return EXACTLY in this format:\n\
        SCORE: <number 0-100>\n\
        STRONG_MATCHES: item1, item2, item3\n\
        GAPS: item1, item2, item3\n\
        STRETCH_AREAS: item1, item2, item3\n\
        NARRATIVE:\n\
        <2-3 paragraph narrative assessment>\n\n\
        Job Title: {}\n\n\
        Job Posting:\n{}\n\n\
        Resume:\n{}",
        title, job_text, resume
    );

    let response = provider.complete(&prompt, 4096)?;

    let mut fit_score = 0.0;
    let mut strong_matches = Vec::new();
    let mut gaps = Vec::new();
    let mut stretch_areas = Vec::new();
    let mut narrative = String::new();
    let mut in_narrative = false;

    for line in response.lines() {
        let line_trimmed = line.trim();

        if in_narrative {
            if !narrative.is_empty() {
                narrative.push('\n');
            }
            narrative.push_str(line);
            continue;
        }

        if let Some(rest) = line_trimmed.strip_prefix("SCORE:") {
            fit_score = rest.trim().parse::<f64>().unwrap_or(0.0);
        } else if let Some(rest) = line_trimmed.strip_prefix("STRONG_MATCHES:") {
            strong_matches = rest
                .split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect();
        } else if let Some(rest) = line_trimmed.strip_prefix("GAPS:") {
            gaps = rest
                .split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect();
        } else if let Some(rest) = line_trimmed.strip_prefix("STRETCH_AREAS:") {
            stretch_areas = rest
                .split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect();
        } else if line_trimmed.starts_with("NARRATIVE:") {
            in_narrative = true;
        }
    }

    Ok(FitResult {
        fit_score,
        strong_matches,
        gaps,
        stretch_areas,
        narrative: narrative.trim().to_string(),
    })
}

#[allow(dead_code)]
pub fn tailor_resume_suggestions(
    provider: &dyn AIProvider,
    resume: &str,
    job_text: &str,
    title: &str,
) -> Result<String> {
    let prompt = format!(
        "You are helping tailor a resume for a specific job. Given the base resume and job posting below, suggest specific improvements:\n\n\
        1. Which skills/experiences from the resume should be emphasized?\n\
        2. What keywords from the job posting should be incorporated?\n\
        3. How should the resume be restructured or reordered for this role?\n\
        4. What should be added or removed?\n\n\
        Provide a clear, actionable summary that can be used to improve the resume for this specific position.\n\n\
        Job Title: {}\n\n\
        Job Posting:\n{}\n\n\
        Base Resume:\n{}",
        title, job_text, resume
    );

    provider.complete(&prompt, 4096)
}

pub fn tailor_resume_full(
    provider: &dyn AIProvider,
    all_resumes: &[(String, String)], // (name, content) pairs
    job_text: &str,
    title: &str,
    employer: Option<&str>,
    output_format: &str,
) -> Result<String> {
    let mut resume_sections = String::new();
    for (i, (name, content)) in all_resumes.iter().enumerate() {
        if i == 0 {
            resume_sections.push_str(&format!("=== PRIMARY RESUME: {} ===\n{}\n\n", name, content));
        } else {
            resume_sections.push_str(&format!(
                "=== ADDITIONAL RESUME: {} ===\n{}\n\n",
                name, content
            ));
        }
    }

    let employer_str = employer.unwrap_or("the employer");
    let format_instruction = match output_format {
        "latex" => "Generate a complete LaTeX document for the resume. Use a clean, professional template with appropriate LaTeX packages. The output should compile directly with pdflatex.",
        _ => "Generate the resume in clean markdown format, suitable for conversion to PDF or other formats.",
    };

    let prompt = format!(
        "You are an expert resume writer. Generate a COMPLETE, TAILORED resume for the job below.\n\n\
        IMPORTANT RULES:\n\
        - Mine ALL provided resumes for relevant experience, skills, and achievements\n\
        - Stay 100% truthful — only use facts from the provided resumes\n\
        - Tailor language, emphasis, and ordering for this specific role\n\
        - Include ALL relevant experience across all resumes — don't omit anything useful\n\
        - {format_instruction}\n\n\
        Job Title: {title}\n\
        Employer: {employer_str}\n\n\
        Job Posting:\n{job_text}\n\n\
        {resume_sections}\n\
        Generate the complete tailored resume now:",
    );

    provider.complete(&prompt, 8192)
}

#[derive(Debug)]
pub struct GlassdoorReviewData {
    pub rating: f64,
    pub title: String,
    pub pros: String,
    pub cons: String,
    pub sentiment: String, // "positive", "negative", "neutral"
    pub review_date: String,
}

#[derive(Debug)]
pub struct GlassdoorResearch {
    pub reviews: Vec<GlassdoorReviewData>,
}

pub fn research_glassdoor(
    provider: &dyn AIProvider,
    employer_name: &str,
) -> Result<GlassdoorResearch> {
    let prompt = format!(
        "Research what employees say about working at \"{employer_name}\" on Glassdoor and similar \
        review sites. Based on your knowledge, generate 5-8 representative employee reviews that \
        reflect the actual reputation and common themes for this company.\n\n\
        For EACH review, return a line in this EXACT format:\n\
        REVIEW: <rating 1.0-5.0> | <sentiment: positive/negative/neutral> | <date YYYY-MM-DD> | <short title> | <pros> | <cons>\n\n\
        RULES:\n\
        - Ratings should reflect the company's actual Glassdoor reputation\n\
        - Include a realistic mix of positive, negative, and neutral reviews\n\
        - Pros and cons should be specific to this company, not generic\n\
        - Dates should be recent (2025-2026)\n\
        - Each field separated by \" | \" (space-pipe-space)\n\
        - If you don't know anything about this company, return exactly: UNKNOWN\n\n\
        Return ONLY REVIEW: lines (or UNKNOWN), nothing else."
    );

    let response = provider.complete(&prompt, 4096)?;

    let trimmed = response.trim();
    if trimmed == "UNKNOWN" || trimmed.is_empty() {
        return Err(anyhow!("No Glassdoor data available for '{}'", employer_name));
    }

    let mut reviews = Vec::new();

    for line in response.lines() {
        let line = line.trim();
        let Some(rest) = line.strip_prefix("REVIEW:") else { continue };
        let parts: Vec<&str> = rest.split(" | ").map(|s| s.trim()).collect();
        if parts.len() < 6 {
            continue;
        }

        let rating = parts[0].parse::<f64>().unwrap_or(3.0).clamp(1.0, 5.0);
        let sentiment = match parts[1] {
            "positive" | "negative" | "neutral" => parts[1].to_string(),
            _ => {
                if rating >= 4.0 { "positive".to_string() }
                else if rating <= 2.0 { "negative".to_string() }
                else { "neutral".to_string() }
            }
        };
        let review_date = parts[2].to_string();
        let title = parts[3].to_string();
        let pros = parts[4].to_string();
        let cons = parts[5].to_string();

        reviews.push(GlassdoorReviewData {
            rating,
            title,
            pros,
            cons,
            sentiment,
            review_date,
        });
    }

    if reviews.is_empty() {
        return Err(anyhow!("Could not parse Glassdoor reviews for '{}'", employer_name));
    }

    Ok(GlassdoorResearch { reviews })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_resolve_model_claude_code() {
        let spec = resolve_model("claude-sonnet").unwrap();
        assert_eq!(spec.model_id, "claude-sonnet-4-5-20250929");
        assert!(matches!(spec.provider, ProviderKind::ClaudeCode));

        let spec = resolve_model("sonnet").unwrap();
        assert_eq!(spec.short_name, "claude-sonnet");

        let spec = resolve_model("opus").unwrap();
        assert_eq!(spec.model_id, "claude-opus-4-6");
        assert!(matches!(spec.provider, ProviderKind::ClaudeCode));

        let spec = resolve_model("haiku").unwrap();
        assert!(matches!(spec.provider, ProviderKind::ClaudeCode));
    }

    #[test]
    fn test_resolve_model_anthropic_api() {
        let spec = resolve_model("api-sonnet").unwrap();
        assert_eq!(spec.model_id, "claude-sonnet-4-5-20250929");
        assert!(matches!(spec.provider, ProviderKind::Anthropic));

        let spec = resolve_model("api-opus").unwrap();
        assert!(matches!(spec.provider, ProviderKind::Anthropic));
    }

    #[test]
    fn test_resolve_model_openai() {
        let spec = resolve_model("gpt-5.2").unwrap();
        assert_eq!(spec.model_id, "gpt-5.2");
        assert!(matches!(spec.provider, ProviderKind::OpenAI));

        let spec = resolve_model("gpt5").unwrap();
        assert_eq!(spec.short_name, "gpt-5.2");

        let spec = resolve_model("gpt-5.2-pro").unwrap();
        assert!(matches!(spec.provider, ProviderKind::OpenAI));

        let spec = resolve_model("gpt-4o").unwrap();
        assert!(matches!(spec.provider, ProviderKind::OpenAI));

        let spec = resolve_model("o3").unwrap();
        assert!(matches!(spec.provider, ProviderKind::OpenAI));
    }

    #[test]
    fn test_resolve_model_unknown() {
        let result = resolve_model("gpt-3");
        assert!(result.is_err());
    }

    #[test]
    fn test_anthropic_provider_api_key() {
        // Test both presence and absence in one test to avoid parallel env var races
        let original = env::var("ANTHROPIC_API_KEY").ok();

        // With key set
        unsafe { env::set_var("ANTHROPIC_API_KEY", "test-key"); }
        let result = AnthropicProvider::new("claude-sonnet-4-5-20250929".to_string());
        assert!(result.is_ok());
        assert_eq!(result.unwrap().model_name(), "claude-sonnet-4-5-20250929");

        // Without key
        unsafe { env::remove_var("ANTHROPIC_API_KEY"); }
        let result = AnthropicProvider::new("claude-sonnet-4-5-20250929".to_string());
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("ANTHROPIC_API_KEY"));

        // Restore
        if let Some(val) = original {
            unsafe { env::set_var("ANTHROPIC_API_KEY", val); }
        }
    }

    #[test]
    fn test_openai_provider_api_key() {
        let original = env::var("OPENAI_API_KEY").ok();

        unsafe { env::remove_var("OPENAI_API_KEY"); }
        let result = OpenAIProvider::new("gpt-4o".to_string());
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("OPENAI_API_KEY"));

        if let Some(val) = original {
            unsafe { env::set_var("OPENAI_API_KEY", val); }
        }
    }

    #[test]
    fn test_parse_weighted_keywords_basic() {
        let result = parse_weighted_keywords("Kubernetes/3, Python/2, dbt/1");
        assert_eq!(result.len(), 3);
        assert_eq!(result[0], ("Kubernetes".to_string(), 3));
        assert_eq!(result[1], ("Python".to_string(), 2));
        assert_eq!(result[2], ("dbt".to_string(), 1));
    }

    #[test]
    fn test_parse_weighted_keywords_no_weight() {
        let result = parse_weighted_keywords("Kubernetes, Python");
        assert_eq!(result.len(), 2);
        assert_eq!(result[0], ("Kubernetes".to_string(), 2));
        assert_eq!(result[1], ("Python".to_string(), 2));
    }

    #[test]
    fn test_parse_weighted_keywords_empty() {
        let result = parse_weighted_keywords("");
        assert!(result.is_empty());
    }

    #[test]
    fn test_parse_weighted_keywords_clamp() {
        let result = parse_weighted_keywords("Kubernetes/5, Python/0, AWS/-10, Docker/10");
        assert_eq!(result.len(), 4);
        assert_eq!(result[0], ("Kubernetes".to_string(), 3));
        assert_eq!(result[1], ("Python".to_string(), 1));
        assert_eq!(result[2], ("AWS".to_string(), 1));
        assert_eq!(result[3], ("Docker".to_string(), 3));
    }

    #[test]
    fn test_parse_weighted_keywords_whitespace() {
        let result = parse_weighted_keywords("  Kubernetes / 3 ,  Python /2  , dbt/ 1  ");
        assert_eq!(result.len(), 3);
        assert_eq!(result[0], ("Kubernetes".to_string(), 3));
        assert_eq!(result[1], ("Python".to_string(), 2));
        assert_eq!(result[2], ("dbt".to_string(), 1));
    }

    #[test]
    fn test_dedup_keywords_no_dupes() {
        let keywords = vec![
            ("Kubernetes".to_string(), 3),
            ("Python".to_string(), 2),
            ("AWS".to_string(), 1),
        ];
        let result = dedup_keywords(keywords);
        assert_eq!(result.len(), 3);
    }

    #[test]
    fn test_dedup_keywords_case_insensitive() {
        let keywords = vec![
            ("AWS".to_string(), 2),
            ("aws".to_string(), 3),
        ];
        let result = dedup_keywords(keywords);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].1, 3);
    }

    #[test]
    fn test_dedup_keywords_empty() {
        let result = dedup_keywords(vec![]);
        assert!(result.is_empty());
    }

    #[test]
    fn test_resolve_model_gpt5_pro_alias() {
        let spec = resolve_model("gpt5-pro").unwrap();
        assert_eq!(spec.model_id, "gpt-5.2-pro");
        assert!(matches!(spec.provider, ProviderKind::OpenAI));
    }

    // --- Mock AIProvider for testing parsing logic ---

    struct MockProvider {
        response: String,
    }

    impl MockProvider {
        fn new(response: &str) -> Self {
            Self { response: response.to_string() }
        }
    }

    impl AIProvider for MockProvider {
        fn complete(&self, _prompt: &str, _max_tokens: u32) -> Result<String> {
            Ok(self.response.clone())
        }
        fn model_name(&self) -> &str { "mock" }
    }

    #[test]
    fn test_analyze_job_returns_response() {
        let provider = MockProvider::new("Analysis: This is a senior role requiring Kubernetes.");
        let result = analyze_job(&provider, "Senior DevOps Engineer needed").unwrap();
        assert!(result.contains("senior role"));
    }

    #[test]
    fn test_extract_keywords_parses_csv() {
        let provider = MockProvider::new("Kubernetes, Python, Terraform, AWS, Docker");
        let result = extract_keywords(&provider, "job text").unwrap();
        assert_eq!(result.len(), 5);
        assert_eq!(result[0], "Kubernetes");
        assert_eq!(result[4], "Docker");
    }

    #[test]
    fn test_extract_keywords_handles_whitespace() {
        let provider = MockProvider::new("  Kubernetes , Python  ,  , Terraform  ");
        let result = extract_keywords(&provider, "job text").unwrap();
        assert_eq!(result.len(), 3);
        assert_eq!(result[0], "Kubernetes");
    }

    #[test]
    fn test_extract_domain_keywords_full_response() {
        let provider = MockProvider::new(
            "TECH: Kubernetes/3, Python/2, dbt/1\n\
             DISCIPLINE: DevOps/3, SRE/2, Agile/1\n\
             CLOUD: AWS/3, Azure/1\n\
             SOFT_SKILL: leadership/3, communication/2\n\
             PROFILE: Tech-heavy infrastructure role."
        );
        let result = extract_domain_keywords(&provider, "job text").unwrap();
        assert_eq!(result.tech.len(), 3);
        assert_eq!(result.tech[0].0, "Kubernetes");
        assert_eq!(result.tech[0].1, 3);
        assert_eq!(result.discipline.len(), 3);
        assert_eq!(result.cloud.len(), 2);
        assert_eq!(result.soft_skill.len(), 2);
        assert_eq!(result.profile, "Tech-heavy infrastructure role.");
    }

    #[test]
    fn test_extract_domain_keywords_cross_domain_dedup() {
        let provider = MockProvider::new(
            "TECH: AWS/3, Python/2\n\
             DISCIPLINE: DevOps/3\n\
             CLOUD: AWS/2\n\
             SOFT_SKILL: leadership/3\n\
             PROFILE: Test."
        );
        let result = extract_domain_keywords(&provider, "job text").unwrap();
        // AWS should only appear in TECH (first seen)
        assert!(result.tech.iter().any(|(k, _)| k == "AWS"));
        assert!(!result.cloud.iter().any(|(k, _)| k.to_lowercase() == "aws"));
    }

    #[test]
    fn test_extract_domain_keywords_empty_response() {
        let provider = MockProvider::new("");
        let result = extract_domain_keywords(&provider, "job text").unwrap();
        assert!(result.tech.is_empty());
        assert!(result.discipline.is_empty());
        assert!(result.cloud.is_empty());
        assert!(result.soft_skill.is_empty());
        assert!(result.profile.is_empty());
    }

    #[test]
    fn test_extract_domain_keywords_partial_response() {
        let provider = MockProvider::new(
            "TECH: Rust/3, Go/2\n\
             PROFILE: Systems programming role."
        );
        let result = extract_domain_keywords(&provider, "job text").unwrap();
        assert_eq!(result.tech.len(), 2);
        assert!(result.discipline.is_empty());
        assert!(result.cloud.is_empty());
        assert!(result.soft_skill.is_empty());
        assert_eq!(result.profile, "Systems programming role.");
    }

    #[test]
    fn test_analyze_fit_parses_response() {
        let provider = MockProvider::new(
            "SCORE: 75\n\
             STRONG_MATCHES: Kubernetes, Python, AWS\n\
             GAPS: Java, Spring Boot\n\
             STRETCH_AREAS: system design, distributed systems\n\
             NARRATIVE:\n\
             Strong fit for this role. The candidate has extensive cloud experience.\n\
             Some gaps in Java ecosystem but transferable skills are solid."
        );
        let result = analyze_fit(&provider, "my resume", "job text", "DevOps Engineer").unwrap();
        assert!((result.fit_score - 75.0).abs() < 0.1);
        assert_eq!(result.strong_matches.len(), 3);
        assert_eq!(result.strong_matches[0], "Kubernetes");
        assert_eq!(result.gaps.len(), 2);
        assert_eq!(result.gaps[0], "Java");
        assert_eq!(result.stretch_areas.len(), 2);
        assert!(result.narrative.contains("Strong fit"));
        assert!(result.narrative.contains("gaps in Java"));
    }

    #[test]
    fn test_analyze_fit_empty_sections() {
        let provider = MockProvider::new(
            "SCORE: 50\n\
             STRONG_MATCHES:\n\
             GAPS:\n\
             STRETCH_AREAS:\n\
             NARRATIVE:\n\
             Average fit."
        );
        let result = analyze_fit(&provider, "resume", "job", "Title").unwrap();
        assert!((result.fit_score - 50.0).abs() < 0.1);
        assert!(result.strong_matches.is_empty());
        assert!(result.gaps.is_empty());
        assert!(result.stretch_areas.is_empty());
        assert!(result.narrative.contains("Average fit"));
    }

    #[test]
    fn test_analyze_fit_bad_score_defaults_zero() {
        let provider = MockProvider::new(
            "SCORE: not-a-number\n\
             STRONG_MATCHES: Python\n\
             GAPS: Java\n\
             STRETCH_AREAS: Go\n\
             NARRATIVE:\n\
             Test."
        );
        let result = analyze_fit(&provider, "resume", "job", "Title").unwrap();
        assert!((result.fit_score - 0.0).abs() < 0.1);
    }

    #[test]
    fn test_tailor_resume_suggestions_returns_response() {
        let provider = MockProvider::new("Emphasize Kubernetes experience. Add more AWS keywords.");
        let result = tailor_resume_suggestions(&provider, "resume", "job text", "DevOps").unwrap();
        assert!(result.contains("Kubernetes"));
    }

    #[test]
    fn test_tailor_resume_full_markdown() {
        let provider = MockProvider::new("# John Doe\n## Experience\n- DevOps at Acme");
        let resumes = vec![("main".to_string(), "John Doe resume content".to_string())];
        let result = tailor_resume_full(&provider, &resumes, "job text", "DevOps", Some("Acme"), "markdown").unwrap();
        assert!(result.contains("John Doe"));
    }

    #[test]
    fn test_tailor_resume_full_latex() {
        let provider = MockProvider::new("\\documentclass{article}\n\\begin{document}\nJohn Doe\n\\end{document}");
        let resumes = vec![
            ("main".to_string(), "primary resume".to_string()),
            ("extra".to_string(), "secondary resume".to_string()),
        ];
        let result = tailor_resume_full(&provider, &resumes, "job text", "DevOps", None, "latex").unwrap();
        assert!(result.contains("\\documentclass"));
    }

    #[test]
    fn test_research_glassdoor_parses_reviews() {
        let provider = MockProvider::new(
            "REVIEW: 4.2 | positive | 2025-06-15 | Great culture | Good WLB, smart peers | Slow promotions\n\
             REVIEW: 2.5 | negative | 2025-03-10 | Burnout city | Good pay | Terrible management, 60hr weeks\n\
             REVIEW: 3.0 | neutral | 2025-01-20 | It's fine | Decent benefits | Nothing special"
        );
        let result = research_glassdoor(&provider, "Acme Corp").unwrap();
        assert_eq!(result.reviews.len(), 3);
        assert!((result.reviews[0].rating - 4.2).abs() < 0.01);
        assert_eq!(result.reviews[0].sentiment, "positive");
        assert_eq!(result.reviews[0].title, "Great culture");
        assert_eq!(result.reviews[0].pros, "Good WLB, smart peers");
        assert_eq!(result.reviews[0].cons, "Slow promotions");
        assert_eq!(result.reviews[1].sentiment, "negative");
        assert!((result.reviews[2].rating - 3.0).abs() < 0.01);
    }

    #[test]
    fn test_research_glassdoor_unknown() {
        let provider = MockProvider::new("UNKNOWN");
        let result = research_glassdoor(&provider, "Mystery Corp");
        assert!(result.is_err());
    }

    #[test]
    fn test_research_glassdoor_empty() {
        let provider = MockProvider::new("");
        let result = research_glassdoor(&provider, "Empty Corp");
        assert!(result.is_err());
    }

    #[test]
    fn test_research_glassdoor_bad_sentiment_inferred() {
        let provider = MockProvider::new(
            "REVIEW: 4.5 | xyz | 2025-01-01 | Title | Pros | Cons\n\
             REVIEW: 1.5 | abc | 2025-01-01 | Title2 | Pros2 | Cons2"
        );
        let result = research_glassdoor(&provider, "Test Corp").unwrap();
        // Rating >= 4.0 with invalid sentiment -> "positive"
        assert_eq!(result.reviews[0].sentiment, "positive");
        // Rating <= 2.0 with invalid sentiment -> "negative"
        assert_eq!(result.reviews[1].sentiment, "negative");
    }

    #[test]
    fn test_research_glassdoor_rating_clamped() {
        let provider = MockProvider::new(
            "REVIEW: 10.0 | positive | 2025-01-01 | Title | Pros | Cons\n\
             REVIEW: -1.0 | negative | 2025-01-01 | Title2 | Pros2 | Cons2"
        );
        let result = research_glassdoor(&provider, "Test Corp").unwrap();
        assert!((result.reviews[0].rating - 5.0).abs() < 0.01);
        assert!((result.reviews[1].rating - 1.0).abs() < 0.01);
    }

    #[test]
    fn test_research_glassdoor_skips_malformed_lines() {
        let provider = MockProvider::new(
            "Some random text\n\
             REVIEW: 4.0 | positive | 2025-01-01 | Title | Pros | Cons\n\
             REVIEW: bad line with too few parts\n\
             Another random line"
        );
        let result = research_glassdoor(&provider, "Test Corp").unwrap();
        assert_eq!(result.reviews.len(), 1);
    }
}
