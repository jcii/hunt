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
            let provider = ClaudeCodeProvider::new(spec.model_id.clone())?;
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
        let client = reqwest::blocking::Client::new();
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
        let output = std::process::Command::new("claude")
            .arg("-p")
            .arg(prompt)
            .arg("--model")
            .arg(&self.model_id)
            .output()
            .context("Failed to run 'claude' CLI")?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(anyhow!("claude CLI failed: {}", stderr));
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
    max_tokens: u32,
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
        let client = reqwest::blocking::Client::new();
        Ok(Self { api_key, model_id, client })
    }
}

impl AIProvider for OpenAIProvider {
    fn complete(&self, prompt: &str, max_tokens: u32) -> Result<String> {
        let request = OpenAIRequest {
            model: self.model_id.clone(),
            max_tokens,
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

pub struct CategorizedKeywords {
    pub mandatory: Vec<String>,
    pub nice_to_have: Vec<String>,
}

pub fn extract_categorized_keywords(
    provider: &dyn AIProvider,
    job_text: &str,
) -> Result<CategorizedKeywords> {
    let prompt = format!(
        "Analyze this job posting and extract technical skills, technologies, and requirements.\n\
        Categorize them as MANDATORY (explicitly required, must-have) or NICE_TO_HAVE (preferred, bonus, nice to have).\n\n\
        Return EXACTLY in this format with no other text:\n\
        MANDATORY: skill1, skill2, skill3\n\
        NICE_TO_HAVE: skill1, skill2, skill3\n\n\
        Job posting:\n{}",
        job_text
    );

    let response = provider.complete(&prompt, 4096)?;

    let mut mandatory = Vec::new();
    let mut nice_to_have = Vec::new();

    for line in response.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix("MANDATORY:") {
            mandatory = rest
                .split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect();
        } else if let Some(rest) = line.strip_prefix("NICE_TO_HAVE:") {
            nice_to_have = rest
                .split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect();
        }
    }

    Ok(CategorizedKeywords {
        mandatory,
        nice_to_have,
    })
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
    fn test_anthropic_provider_requires_api_key() {
        let original = env::var("ANTHROPIC_API_KEY").ok();
        unsafe { env::remove_var("ANTHROPIC_API_KEY"); }

        let result = AnthropicProvider::new("claude-sonnet-4-5-20250929".to_string());

        if let Some(val) = original {
            unsafe { env::set_var("ANTHROPIC_API_KEY", val); }
        }

        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(err_msg.contains("ANTHROPIC_API_KEY"));
    }

    #[test]
    fn test_anthropic_provider_with_api_key() {
        unsafe { env::set_var("ANTHROPIC_API_KEY", "test-key"); }

        let result = AnthropicProvider::new("claude-sonnet-4-5-20250929".to_string());
        assert!(result.is_ok());
        assert_eq!(result.unwrap().model_name(), "claude-sonnet-4-5-20250929");

        unsafe { env::remove_var("ANTHROPIC_API_KEY"); }
    }

    #[test]
    fn test_openai_provider_requires_api_key() {
        let original = env::var("OPENAI_API_KEY").ok();
        unsafe { env::remove_var("OPENAI_API_KEY"); }

        let result = OpenAIProvider::new("gpt-4o".to_string());

        if let Some(val) = original {
            unsafe { env::set_var("OPENAI_API_KEY", val); }
        }

        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(err_msg.contains("OPENAI_API_KEY"));
    }
}
