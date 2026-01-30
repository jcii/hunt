use anyhow::{anyhow, Context, Result};
use serde::{Deserialize, Serialize};
use std::env;

const ANTHROPIC_API_URL: &str = "https://api.anthropic.com/v1/messages";
const DEFAULT_MODEL: &str = "claude-sonnet-4-5-20250929";

#[derive(Debug, Serialize)]
struct Message {
    role: String,
    content: String,
}

#[derive(Debug, Serialize)]
struct AnthropicRequest {
    model: String,
    max_tokens: u32,
    messages: Vec<Message>,
}

#[derive(Debug, Deserialize)]
struct ContentBlock {
    #[allow(dead_code)]
    #[serde(rename = "type")]
    content_type: String,
    text: String,
}

#[derive(Debug, Deserialize)]
struct AnthropicResponse {
    content: Vec<ContentBlock>,
}

#[derive(Debug)]
pub struct AIClient {
    api_key: String,
    client: reqwest::blocking::Client,
}

impl AIClient {
    pub fn new() -> Result<Self> {
        let api_key = env::var("ANTHROPIC_API_KEY")
            .context("ANTHROPIC_API_KEY environment variable not set. Set it with: export ANTHROPIC_API_KEY=your-key-here")?;

        let client = reqwest::blocking::Client::new();

        Ok(AIClient { api_key, client })
    }

    fn call_api(&self, prompt: &str) -> Result<String> {
        let request = AnthropicRequest {
            model: DEFAULT_MODEL.to_string(),
            max_tokens: 4096,
            messages: vec![Message {
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
                "API request failed with status {}: {}",
                status,
                error_text
            ));
        }

        let api_response: AnthropicResponse = response
            .json()
            .context("Failed to parse API response")?;

        api_response
            .content
            .first()
            .map(|block| block.text.clone())
            .ok_or_else(|| anyhow!("No content in API response"))
    }

    pub fn extract_keywords(&self, job_text: &str) -> Result<Vec<String>> {
        let prompt = format!(
            "Analyze this job posting and extract key technical skills, technologies, and requirements. Return ONLY a comma-separated list of keywords, no explanations.\n\nJob posting:\n{}",
            job_text
        );

        let response = self.call_api(&prompt)?;

        let keywords: Vec<String> = response
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();

        Ok(keywords)
    }

    pub fn analyze_job(&self, job_text: &str) -> Result<String> {
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

        self.call_api(&prompt)
    }

    pub fn tailor_resume(&self, resume_content: &str, job_text: &str, job_title: &str) -> Result<String> {
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
            job_title, job_text, resume_content
        );

        self.call_api(&prompt)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ai_client_creation_without_api_key() {
        // Save original value
        let original = env::var("ANTHROPIC_API_KEY").ok();

        // Clear the environment variable for this test
        unsafe {
            env::remove_var("ANTHROPIC_API_KEY");
        }

        let result = AIClient::new();

        // Restore original value before assertions
        if let Some(val) = original {
            unsafe {
                env::set_var("ANTHROPIC_API_KEY", val);
            }
        }

        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(err_msg.contains("ANTHROPIC_API_KEY"));
    }

    #[test]
    fn test_ai_client_creation_with_api_key() {
        // Set a dummy API key for this test
        unsafe {
            env::set_var("ANTHROPIC_API_KEY", "test-key");
        }

        let result = AIClient::new();
        assert!(result.is_ok());

        // Clean up
        unsafe {
            env::remove_var("ANTHROPIC_API_KEY");
        }
    }
}
